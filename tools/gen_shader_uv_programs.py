#!/usr/bin/env python3
"""Generates the oracle `oracles.dxc.elden/shader-uv-programs.json`: for each Elden Ring surface shader, which vertex-input
texture-coordinate lanes the coordinates of the textures its pixel program samples are computed from.

Input: the shader bundles' programs, each `<stem>/<stem>_<pass>.ppo` (pixel) and `.vpo` (vertex) a DXBC container holding
DXIL (the members of `shader/shaderbdle.shaderbdlebnd.dcx`'s bundles, written out as stored), and Microsoft's `dxc.exe`
(Windows SDK 10.0.26100.0), whose `-dumpbin` disassembles a container to DXIL text.

    python -I tools/gen_shader_uv_programs.py --dxc PATH/TO/dxc.exe --shaders PATH/TO/shaderbdle --out shader-uv-programs.json

For a stem it takes the Gbuf pair of programs (the Fwd pair where there is no Gbuf one; a stem with neither is left out) and
  * in the pixel program finds every `dx.op.sample*` call, resolves its texture handle to the resource name
    (`<stem>_snp_<k>_Texture2D_<n>`) and walks the coordinate operands back through the SSA graph to `loadInput` leaves
    (pixel input element and lane), stopping at constant-buffer loads (the tiling and offset constants) and at another
    texture's sampled value (`via`);
  * in the vertex program walks each `storeOutput` value back to its `loadInput` leaves (vertex input element and lane);
  * joins the two on the interpolant (semantic, index, lane).
A vertex input `TEXCOORDi` is one UV slot, except an `int xyzw` one (two packed int16 pairs: a `UVPair` member), which is two;
the slots number consecutively over the declared `TEXCOORD` inputs by index. A texture's `slots` are the UV slots its
coordinate depends on, `also` the other vertex-input semantics it depends on (position, normal, tangent, colour: a projected
or normal-offset coordinate), `via` whether it depends on another texture's value (a parallax or distortion offset).
"""
import argparse
import json
import os
import re
import subprocess

LANES = "xyzw"
SAMPLE_OPS = ("sample", "sampleBias", "sampleLevel", "sampleGrad", "sampleCmp", "sampleCmpLevelZero", "textureGather", "textureGatherCmp")
DXC = None


def dump(path):
    out = subprocess.run([DXC, "-dumpbin", path], capture_output=True, text=True, encoding="utf-8", errors="replace")
    if out.returncode != 0:
        raise RuntimeError(f"dxc {path}: {out.stderr[:200]}")
    return out.stdout


class Program:
    def __init__(self, text):
        self.inputs = []   # first Input signature table
        self.outputs = []  # Output signature table
        self.bindings = {}  # (class letter, id) -> name
        self.defs = {}      # %n -> (kind, rhs)
        self.stores = []    # (sigid, col, valueref)
        self.parse(text)

    def parse(self, text):
        lines = text.split("\n")
        section = None
        seen_in = False
        for ln in lines:
            if ln.startswith("; Input signature:") and not seen_in:
                section, seen_in = "in", True
                continue
            if ln.startswith("; Output signature:") and section in ("in", None) and not self.outputs:
                section = "out"
                continue
            if ln.startswith("; Resource Bindings:"):
                section = "res"
                continue
            if ln.startswith("; ViewId state") or ln.startswith("target datalayout"):
                section = None
            if section in ("in", "out") and ln.startswith("; ") and not ln.startswith("; Name") and not ln.startswith("; ---"):
                t = ln[2:].split()
                if len(t) >= 6 and t[1].isdigit() and all(c in LANES for c in t[2]):
                    el = {"name": t[0], "index": int(t[1]), "mask": t[2], "fmt": t[5], "used": t[6] if len(t) > 6 else ""}
                    (self.inputs if section == "in" else self.outputs).append(el)
                elif section in ("in", "out") and ln.strip() == ";" and (self.inputs if section == "in" else self.outputs):
                    section = None
            elif section == "res" and ln.startswith("; ") and not ln.startswith("; Name") and not ln.startswith("; ---"):
                t = ln[2:].split()
                m2 = re.fullmatch(r"([A-Z]+)(\d+)", t[4]) if len(t) >= 7 else None
                if m2:
                    self.bindings[({"CB": "C"}.get(m2.group(1), m2.group(1)), int(m2.group(2)))] = (t[0], t[1], t[3])
            m = re.match(r"\s*%(\d+) = (.*?)(  ;.*)?$", ln)
            if m:
                self.defs[int(m.group(1))] = m.group(2)
            else:
                m = re.match(r"\s*call void @dx\.op\.storeOutput\.\w+\((.*?)\)(  ;.*)?$", ln)
                if m:
                    a = [x.strip() for x in m.group(1).split(", ")]
                    # (i32 5, i32 sigId, i32 row, i8 col, <ty> value)
                    sig = int(a[1].split()[-1])
                    col = int(a[3].split()[-1])
                    val = a[4].split()[-1]
                    self.stores.append((sig, col, val))

    def lane_of(self, el, col):
        """absolute lane of a load/store at relative column `col` of element `el`"""
        return LANES.index(el["mask"][0]) + col

    # ---- SSA walking ----
    def args(self, rhs):
        m = re.search(r"@(dx\.op\.\w+)(?:\.\w+)?\((.*)\)\s*$", rhs)
        if not m:
            return None, None
        return m.group(1), [x.strip() for x in m.group(2).split(", ")]

    def trace(self, ref, memo=None):
        """leaves reachable from value `ref` ('%n' or a constant): ({('in', sigid, col)}, {texture resource names via}, flags)"""
        leaves, via, flags = set(), set(), set()
        seen = set()
        stack = [ref]
        while stack:
            r = stack.pop()
            if not r.startswith("%") or not r[1:].isdigit():
                continue
            n = int(r[1:])
            if n in seen:
                continue
            seen.add(n)
            rhs = self.defs.get(n)
            if rhs is None:
                continue
            op, a = self.args(rhs)
            if op == "dx.op.loadInput":
                leaves.add(("in", int(a[1].split()[-1]), int(a[3].split()[-1])))
                continue
            if op == "dx.op.createHandle":
                continue
            if op in ("dx.op.cbufferLoadLegacy", "dx.op.cbufferLoad"):
                flags.add("cbuf")
                continue
            if op is not None and op[len("dx.op."):] in SAMPLE_OPS:
                via.add(self.resource_of(a[1]))
                continue
            if op is not None and op.startswith("dx.op.textureLoad"):
                via.add(self.resource_of(a[1]) + ":load")
                continue
            stack.extend("%" + x for x in re.findall(r"%(\d+)\b", rhs))
        return leaves, via, flags

    def resource_of(self, handle):
        h = handle.split()[-1]
        rhs = self.defs.get(int(h[1:])) if h.startswith("%") and h[1:].isdigit() else None
        if rhs is None:
            return "?"
        op, a = self.args(rhs)
        if op != "dx.op.createHandle":
            return "?"
        cls = {0: "T", 1: "U", 2: "C", 3: "S"}.get(int(a[1].split()[-1]), "?")
        rid = int(a[2].split()[-1])
        b = self.bindings.get((cls, rid))
        return b[0] if b else f"{cls}{rid}"

    def samples(self):
        """[(op, texture name, sampler name, coordinate refs)]"""
        found = []
        for n, rhs in self.defs.items():
            op, a = self.args(rhs)
            if op is None or op[len("dx.op."):] not in SAMPLE_OPS:
                continue
            coords = [x.split()[-1] for x in a[3:7]]
            coords = [c for c in coords if c != "undef"]
            found.append((op[len("dx.op."):], self.resource_of(a[1]), self.resource_of(a[2]), coords))
        return found


def element(prog, sig, which):
    els = prog.inputs if which == "in" else prog.outputs
    return els[sig] if sig < len(els) else None


def analyse(ps_text, vs_text):
    ps, vs = Program(ps_text), Program(vs_text)
    # vertex program: interpolant (name, index, lane) -> vertex input leaves
    vs_out = {}
    for sig, col, val in vs.stores:
        el = element(vs, sig, "out")
        if el is None:
            continue
        lane = vs.lane_of(el, col)
        leaves, via, flags = vs.trace(val)
        srcs = set()
        for _, isig, icol in leaves:
            iel = element(vs, isig, "in")
            if iel is not None:
                srcs.add((iel["name"], iel["index"], vs.lane_of(iel, icol)))
        vs_out[(el["name"], el["index"], lane)] = (sorted(srcs), sorted(via), sorted(flags))
    result = {}
    for kind, tex, smp, coords in ps.samples():
        ps_leaves, via, flags = set(), set(), set()
        for c in coords:
            l, v, f = ps.trace(c)
            ps_leaves |= l
            via |= v
            flags |= f
        interp = set()
        verts = set()
        unlinked = set()
        for _, isig, icol in ps_leaves:
            el = element(ps, isig, "in")
            if el is None:
                continue
            key = (el["name"], el["index"], ps.lane_of(el, icol))
            interp.add(key)
            if key in vs_out:
                verts.update(vs_out[key][0])
            else:
                unlinked.add(key)
        entry = result.setdefault(tex, {"samplers": [], "interpolants": set(), "vertex": set(), "unlinked": set(), "via": set(), "flags": set(), "ops": set()})
        entry["samplers"].append(smp)
        entry["interpolants"] |= interp
        entry["vertex"] |= verts
        entry["unlinked"] |= unlinked
        entry["via"] |= via
        entry["flags"] |= flags
        entry["ops"].add(kind)
    for e in result.values():
        for k in ("interpolants", "vertex", "unlinked", "via", "flags", "ops"):
            e[k] = sorted(e[k])
        e["samplers"] = sorted(set(e["samplers"]))
    vs_inputs = [{"name": e["name"], "index": e["index"], "mask": e["mask"], "fmt": e["fmt"], "used": e["used"]} for e in vs.inputs]
    return result, vs_inputs


def slot_table(vs_inputs):
    """[(TEXCOORD index, first UV slot, slots)] over the vertex program's declared TEXCOORD inputs"""
    out, slot = [], 0
    for el in sorted((e for e in vs_inputs if e["name"] == "TEXCOORD"), key=lambda e: e["index"]):
        width = 2 if (el["fmt"] == "int" and el["mask"] == "xyzw") else 1
        out.append((el["index"], slot, width))
        slot += width
    return out, slot


def uv_slot(table, index, lane):
    for idx, start, width in table:
        if idx == index:
            return start + lane // 2 if width == 2 else start
    return None


def reduce(textures, vs_inputs):
    table, declared = slot_table(vs_inputs)
    out = {}
    for tex, e in textures.items():
        m = re.search(r"Texture2D_(\d+)$", tex)
        if not m:
            continue
        verts = [tuple(v) for v in e["vertex"]]
        slots = sorted({uv_slot(table, i, lane) for (name, i, lane) in verts if name == "TEXCOORD"} - {None})
        also = sorted({name for (name, i, lane) in verts if name != "TEXCOORD"})
        out[m.group(1)] = {"slots": slots, "also": also, "via": bool(e["via"])}
    return declared, out


def main():
    global DXC
    ap = argparse.ArgumentParser()
    ap.add_argument("--dxc", required=True)
    ap.add_argument("--shaders", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    DXC = args.dxc
    stems = {}
    for stem in sorted(os.listdir(args.shaders)):
        d = os.path.join(args.shaders, stem)
        files = set(os.listdir(d))
        pair = next((p for p in ("Gbuf", "Fwd") if f"{stem}_{p}.ppo" in files and f"{stem}_{p}.vpo" in files), None)
        if pair is None:
            continue
        textures, vs_inputs = analyse(dump(os.path.join(d, f"{stem}_{pair}.ppo")), dump(os.path.join(d, f"{stem}_{pair}.vpo")))
        declared, reduced = reduce(textures, vs_inputs)
        stems[stem] = {"pair": pair, "uv_slots": declared, "textures": reduced}
    with open(args.out, "w", encoding="utf-8", newline="\n") as f:
        json.dump({"source": "Microsoft DXC 10.0.26100.0 -dumpbin of the Gbuf (Fwd) pixel and vertex programs of Elden Ring's shader bundles; tools/gen_shader_uv_programs.py", "stems": stems}, f, indent=0, sort_keys=True)
        f.write("\n")


if __name__ == "__main__":
    main()
