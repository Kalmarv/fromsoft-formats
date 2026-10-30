#!/usr/bin/env python3
"""Generates src/fxr/actions.rs: one typed struct per FXR action type that occurs in Sekiro's or Elden Ring's installed
files, with the parameter names, their order and their kinds in each layout the files use.

Input: the fxr project's action data (`dist/actions.json` of EvenTorset/fxr, Unlicense: the names of an action's
properties and fields, their order in each game's files, their types and defaults). The project is the source of the
names, except where TRACED_NAMES gives one that Elden Ring's own code pins (what the game does with the value); the
layout of the lists in the files is checked against them by tests/fxr_actions_census.rs on every action of both installs.
The output is identical to the committed file for the package versions 32.1.2 and 32.1.3, after rustfmt.

    python tools/gen_fxr_actions.py PATH/TO/actions.json > src/fxr/actions.rs

The action types are the ones the installs hold (OCCURRING, from the census of all 15,068 files). Three layouts are
generated per type: Sekiro's, Elden Ring's and Dark Souls III's (a few Sekiro files keep effects in that layout).
"""
import json
import re
import sys
import textwrap

OCCURRING = [1, 11, 15, 34, 35, 36, 46, 55, 60, 64, 65, 75, 83, 84, 105, 106, 113, 120, 121, 122, 123, 128, 129, 130, 131,
             132, 133, 199, 200, 201, 300, 301, 399, 400, 401, 402, 403, 404, 405, 500, 501, 502, 503, 600, 601, 602, 603,
             604, 605, 606, 607, 608, 609, 700, 701, 702, 731, 732, 733, 734, 10000, 10001, 10012, 10013, 10014, 10015,
             10100, 10300, 10301, 10400, 10500, 11000]
# the layouts, in the order the generated code indexes them: the two games, then Dark Souls III's
LAYOUTS = ["Sekiro", "EldenRing", "DarkSouls3"]
KEYWORDS = {"as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
            "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
            "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box",
            "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try"}
FIELD_WIDTH = {"boolean": 1, "integer": 1, "float": 1, "vector2": 2, "vector3": 3, "vector4": 4}
FIELD_RUST = {"boolean": "bool", "integer": "i32", "float": "f32", "vector2": "[f32; 2]", "vector3": "[f32; 3]",
              "vector4": "[f32; 4]"}
METHOD = {"boolean": "boolean", "integer": "int", "float": "float", "vector2": "vec2", "vector3": "vec3", "vector4": "vec4"}
WORD_KIND = {"boolean": "Bool", "integer": "Int", "float": "Float"}
# fields whose library type disagrees with what every installed file stores (found by tests/fxr_actions_census.rs);
# the value is the kind the files agree with
FIELD_OVERRIDES = {}
# names for parameters the fxr project leaves unnamed, from what Elden Ring 1.17.3 does with them: the action's fields
# copied word for word to the force field it creates, and the force the cloth gets from that field. Keyed by action type
# and the project's name; the value is the name used here and what the game does with it.
TRACED_NAMES = {
    10300: {
        "unk_ds3_f1_26": ("distanceFalloff", "When non-zero, the wind fades with the distance from the field's origin "
                          "along its direction: the factor is `1 - |distance| / distanceFalloffRadius`, clamped to 0..1."),
        "unk_ds3_f1_27": ("distanceFalloffRadius", "The distance along the wind's direction at which "
                          "`distanceFalloff` has faded the wind to 0 (at least 0.0001)."),
        "unk_ds3_f1_30": ("local", "When non-zero, a cloth samples the field at each particle; when 0 (and "
                          "`unk_ds3_f1_31` is 0), it samples the field once at the centre of the cloth's bounds and "
                          "applies that wind to every particle."),
        "unk_ds3_f1_40": ("waveLength", "The wind's strength follows a sine wave travelling along the wind's "
                          "direction, one cycle per `waveLength` metres (at least 0.001) and one per second: "
                          "`sin(2 pi / waveLength * (t - distance))`."),
        "unk_ds3_f1_41": ("waveOffset", "Added to the wave's sine before it is halved and raised by a half: the "
                          "wave's strength is `((sin + waveOffset) * 0.5) * 0.5 + 0.5` of the force."),
        "unk_ds3_f1_47": ("weight", "Multiplies the wind the field gives a cloth; a field of weight 0 is skipped."),
        "unk_sdt_f1_54": ("gust", "The gusts' speed along the wind's direction, in metres per second, and a factor "
                          "of their strength; gusts are off when it is not above 0."),
        "unk_sdt_f1_55": ("gustLength", "The gusts' wavelength along the wind (at least 0.001), the across-wind "
                          "wobble's wavelength (twice it) and the wobble's amplitude (at most 5)."),
        "unk_sdt_f1_56": ("gustScale", "Multiplies the gusts' strength."),
        "unk_sdt_f1_57": ("gustOffset", "Added to the gusts' sine before it is halved, as `waveOffset` is to the "
                          "wave's."),
        "unk_sdt_f1_58": ("gustRise", "The time a gust takes to rise to full strength, as part of the gust "
                          "period `gustRise + gustFall + gustRest`."),
        "unk_ds3_f1_49": ("gustFall", "The time a gust takes to fall from full strength, as part of the gust "
                          "period."),
        "unk_ds3_f1_50": ("gustRest", "The calm between gusts, as part of the gust period."),
    },
}


def apply_traced_names(acts):
    """Renames the parameters TRACED_NAMES names, in the action's properties and in every layout's lists."""
    for t, names in TRACED_NAMES.items():
        a = acts[t]
        props = a.setdefault("properties", {})
        for old, (new, doc) in names.items():
            if old not in props or new in props:
                raise SystemExit(f"{t}: cannot rename {old} to {new}")
            meta = dict(props.pop(old))
            meta["traced"] = (old, doc)
            props[new] = meta
            for lay in (a.get("structure") or {}).values():
                for lst in ("properties1", "properties2", "fields1", "fields2"):
                    if lay and lay.get(lst):
                        lay[lst] = [new if n == old else n for n in lay[lst]]


def snake(name):
    s = re.sub(r"(?<=[a-z0-9])([A-Z])", r"_\1", name).lower()
    return s + "_" if s in KEYWORDS else s


def field_type(meta, t, n):
    if (t, n) in FIELD_OVERRIDES:
        return FIELD_OVERRIDES[(t, n)]
    k = meta.get("fieldType")
    if isinstance(k, dict):
        k = k.get("EldenRing")
    return k or "integer"


def lit_float(v):
    v = float(v)
    if v == int(v) and abs(v) < 1e9:
        return f"{int(v)}.0"
    return repr(v)


def match_arms(values):
    """`match which { 0 => a, 1 | 2 => b }` over the three layouts: equal values merged, the last arm `_`."""
    arms = {}
    for i, x in enumerate(values):
        arms.setdefault(x, []).append(i)
    items = sorted(arms.items(), key=lambda kv: kv[1][0])
    parts = []
    for j, (x, idx) in enumerate(items):
        pat = "_" if j == len(items) - 1 else " | ".join(str(i) for i in idx)
        parts.append(f"{pat} => {x}")
    return "match which { " + ", ".join(parts) + " }"


def main(path):
    data = json.load(open(path, encoding="utf-8"))
    acts = {a["type"]: a for a in data}
    apply_traced_names(acts)
    out = []
    w = out.append
    w("//! Generated by `tools/gen_fxr_actions.py` from the fxr project's action data (EvenTorset/fxr, Unlicense). Do not edit.")
    w("//!")
    w("//! One struct per action type that occurs in an installed file, holding every parameter the project names in Sekiro's,")
    w("//! Elden Ring's or Dark Souls III's layout of it, in its type and with the project's default for a parameter the")
    w("//! stored lists are too short to hold (the lists of a newer game end with parameters the older files lack). `Extra`")
    w("//! keeps what the struct cannot name: the stored counts, the words and properties past the named ones, the field lists")
    w("//! and the action's flag bytes.")
    w("#![allow(clippy::too_many_lines, clippy::excessive_precision, clippy::approx_constant)]")
    w("")
    w("use super::typed::{Extra, List, Param, Params, Place, WordKind};")
    w("use super::enums::PropertyArgument;")
    w("use super::resources::{ResourceKind, ResourceRef, TextureKind};")
    w("use super::{Action, Property, Result};")
    w("use crate::Game;")
    w("")
    variants = []
    arguments = []
    for t in OCCURRING:
        a = acts.get(t)
        if a is None:
            raise SystemExit(f"action {t} is not in the data")
        name = a["name"]
        props = a.get("properties", {})
        struct = {lay: (a.get("structure") or {}).get(lay) or {} for lay in LAYOUTS}

        def plist(lay):
            return (struct[lay].get("properties1") or []), (struct[lay].get("properties2") or [])

        # parameters in the order: ER's lists, then Sekiro's extras, then Dark Souls III's
        members = []
        have = set()
        for lst in ("properties1", "properties2", "fields1", "fields2"):
            for lay in ("EldenRing", "Sekiro", "DarkSouls3"):
                for n in struct[lay].get(lst) or []:
                    if n in have:
                        continue
                    have.add(n)
                    members.append(n)
        is_prop = {n: any(n in plist(lay)[0] or n in plist(lay)[1] for lay in LAYOUTS) for n in members}
        kinds = {}
        for n in members:
            if not is_prop[n]:
                k = field_type(props.get(n, {}), t, n)
                if k not in FIELD_WIDTH:
                    raise SystemExit(f"{t} {n}: field type {k}")
                kinds[n] = k
        rust = {n: snake(n) for n in members}
        if len(set(rust.values())) != len(members):
            raise SystemExit(f"{t} {name}: duplicate names")
        places = {lay: {} for lay in LAYOUTS}
        counts = {}
        for lay in LAYOUTS:
            st = struct[lay]
            p1, p2 = plist(lay)
            for i, n in enumerate(p1):
                places[lay][n] = f"Place::P1({i})"
            for i, n in enumerate(p2):
                places[lay][n] = f"Place::P2({i})"
            words = []
            for fl, lname in (("fields1", "F1"), ("fields2", "F2")):
                off = 0
                for n in st.get(fl) or []:
                    k = field_type(props.get(n, {}), t, n)
                    if k not in FIELD_WIDTH:
                        raise SystemExit(f"{t} {n}: field type {k}")
                    if is_prop[n]:
                        if k not in WORD_KIND:
                            raise SystemExit(f"{t} {n}: a property stored as a {k} word")
                        places[lay][n] = f"Place::Word(List::{lname}, {off}, WordKind::{WORD_KIND[k]})"
                    else:
                        places[lay][n] = f"Place::{lname}({off})"
                    off += FIELD_WIDTH[k]
                words.append(off)
            counts[lay] = [len(p1), len(p2), words[0], words[1]]
        w(f"/// Action {t}, `{name}` (slot {a.get('slot')}).")
        w("#[derive(Clone, Debug, PartialEq)]")
        w(f"pub struct {name} {{")
        for n in members:
            meta = props.get(n, {})
            if is_prop[n]:
                c = meta.get("components", 1)
                w(f"    /// Property `{n}` ({c} component{'s' if c != 1 else ''}).")
                w(f"    pub {rust[n]}: Property,")
            elif "traced" in meta:
                old, doc = meta["traced"]
                for line in textwrap.wrap(f"Field `{n}` (the fxr project's `{old}`): {doc}", 110 - len("    /// ")):
                    w(f"    /// {line}")
                w(f"    pub {rust[n]}: {FIELD_RUST[kinds[n]]},")
            else:
                w(f"    /// Field `{n}`.")
                w(f"    pub {rust[n]}: {FIELD_RUST[kinds[n]]},")
        w("    /// What the struct does not name.")
        w("    pub extra: Extra,")
        w("}")
        w("")
        w(f"impl {name} {{")
        w("    /// The action type this struct reads.")
        w(f"    pub const ID: i16 = {t};")
        w("")
        w(f"    /// Reads an action of type {t}: in the layout of `game` if the stored lists fit it, else in Dark Souls III's.")
        w("    pub fn read(a: &Action, game: Game) -> Result<Self> {")
        w(f"        if a.id != {t} {{")
        w(f"            return Err(super::typed::wrong_type(a, {t}));")
        w("        }")
        w("        let w = Params::new(a);")
        cs = ", ".join(str(counts[lay]) for lay in LAYOUTS)
        layout_line = len(out)
        w("")
        w("        Ok(Self {")
        for n in members:
            meta = props.get(n, {})
            d = meta.get("default")
            if is_prop[n]:
                comps = meta.get("components", 1)
                if d is None:
                    d = [0.0] * comps
                if not isinstance(d, list):
                    d = [d]
                d = (list(d) + [d[-1]] * comps)[:comps]
                dflt = "&[" + ", ".join(lit_float(x) for x in d) + "]"
                method = "property"
            else:
                k = kinds[n]
                if d is None:
                    d = {"boolean": False, "integer": 0, "float": 0.0}.get(k, [0.0] * FIELD_WIDTH[k])
                if k == "boolean":
                    dflt = "true" if d else "false"
                elif k == "integer":
                    dflt = str(int(d))
                elif k == "float":
                    dflt = lit_float(d)
                else:
                    dd = d if isinstance(d, list) else [d] * FIELD_WIDTH[k]
                    dflt = "[" + ", ".join(lit_float(x) for x in dd) + "]"
                method = METHOD[k]
            ats = [f"Some({places[lay][n]})" if n in places[lay] else "None" for lay in LAYOUTS]
            expr = ats[0] if len(set(ats)) == 1 else match_arms(ats)
            w(f"            {rust[n]}: w.{method}({expr}, {dflt}, \"{n}\")?,")
        cn = [str(counts[lay]) for lay in LAYOUTS]
        w(f"            extra: w.extra({cn[0] if len(set(cn)) == 1 else match_arms(cn)}),")
        w("        })")
        used = any("which" in line for line in out[layout_line:])
        bind = "let which = " if used else ""
        out[layout_line] = f"        {bind}w.layout(match game {{ Game::Sekiro => 0, Game::EldenRing => 1 }}, &[{cs}])?;"
        w("    }")
        enum_members = [n for n in members if props.get(n, {}).get("enum")]
        w("")
        w("    /// The parameters the fxr project types as enums with their stored value (`None` for a property that is not a")
        w("    /// constant): the parameter, the enum and the value.")
        w("    pub fn enum_values(&self) -> Vec<(&'static str, &'static str, Option<i32>)> {")
        if not enum_members:
            w("        Vec::new()")
        else:
            w("        vec![")
            for n in enum_members:
                enum = props[n]["enum"]
                val = f"self.{rust[n]}.as_integer()" if is_prop[n] else f"Some(self.{rust[n]})"
                w(f'            ("{n}", "{enum}", {val}),')
            w("        ]")
        w("    }")
        w("")
        res_members = [n for n in members if props.get(n, {}).get("resource") in ("texture", "model", "anibnd", "sound")]
        w("    /// The files this action refers to by number: the parameters the fxr project marks as texture, model, animation")
        w("    /// or sound references, with their value (`None` for a property that is not a constant).")
        w("    pub fn resources(&self) -> Vec<ResourceRef> {")
        if not res_members:
            w("        Vec::new()")
        else:
            w("        vec![")
            for n in res_members:
                meta = props[n]
                kind = {"texture": "ResourceKind::Texture(TextureKind::%s)" % {"a": "Albedo", "n": "Normal", "3m": "Specular", "d": "Detail"}[meta.get("textureType", "a")],
                        "model": "ResourceKind::Model", "anibnd": "ResourceKind::Animation", "sound": "ResourceKind::Sound"}[meta["resource"]]
                val = f"self.{rust[n]}.as_integer()" if is_prop[n] else f"Some(self.{rust[n]})"
                w(f'            ResourceRef {{ kind: {kind}, param: "{n}", id: {val} }},')
            w("        ]")
        w("    }")
        w("")
        w("    /// Every named parameter with its value, under the name the fxr project gives it, in the struct's order.")
        w("    pub fn params(&self) -> Vec<(&'static str, Param<'_>)> {")
        if not members:
            w("        Vec::new()")
        else:
            w("        vec![")
            for n in members:
                if is_prop[n]:
                    v = f"Param::Property(&self.{rust[n]})"
                else:
                    k = kinds[n]
                    v = {"boolean": f"Param::Bool(self.{rust[n]})", "integer": f"Param::Int(self.{rust[n]})",
                         "float": f"Param::Float(self.{rust[n]})"}.get(k, f"Param::Vector(self.{rust[n]}.to_vec())")
                w(f'            ("{n}", {v}),')
            w("        ]")
        w("    }")
        w("}")
        w("")
        for n in members:
            arg = props.get(n, {}).get("argument")
            if is_prop[n] and arg:
                arguments.append((t, n, arg))
        variants.append((t, name))
    w("/// An action of any type that occurs in an installed file, typed.")
    w("#[derive(Clone, Debug, PartialEq)]")
    w("#[allow(clippy::large_enum_variant, reason = \"the variants differ by the number of parameters the project names; boxing them would cost an allocation per action\")]")
    w("pub enum Typed {")
    for t, name in variants:
        w(f"    /// Action {t}.")
        w(f"    {name}({name}),")
    w("}")
    w("")
    w("impl Typed {")
    w("    /// The typed form of `a`, or an error naming its type when the action type is not one the installs hold.")
    w("    pub fn read(a: &Action, game: Game) -> Result<Typed> {")
    w("        Ok(match a.id {")
    for t, name in variants:
        w(f"            {t} => Typed::{name}({name}::read(a, game)?),")
    w("            _ => return Err(super::typed::unsupported(a)),")
    w("        })")
    w("    }")
    w("")
    w("    /// The action's type number.")
    w("    pub fn id(&self) -> i16 {")
    w("        match self {")
    for t, name in variants:
        w(f"            Typed::{name}(_) => {t},")
    w("        }")
    w("    }")
    w("")
    w("    /// The action's name in the fxr project.")
    w("    pub fn name(&self) -> &'static str {")
    w("        match self {")
    for t, name in variants:
        w(f"            Typed::{name}(_) => \"{name}\",")
    w("        }")
    w("    }")
    w("")
    w("    /// The parameters typed as enums, with their stored values (see each struct's `enum_values`).")
    w("    pub fn enum_values(&self) -> Vec<(&'static str, &'static str, Option<i32>)> {")
    w("        match self {")
    for t, name in variants:
        w(f"            Typed::{name}(x) => x.enum_values(),")
    w("        }")
    w("    }")
    w("")
    w("    /// Every named parameter with its value (see each struct's `params`).")
    w("    pub fn params(&self) -> Vec<(&'static str, Param<'_>)> {")
    w("        match self {")
    for t, name in variants:
        w(f"            Typed::{name}(x) => x.params(),")
    w("        }")
    w("    }")
    w("")
    w("    /// The files the action refers to by number (see each struct's `resources`).")
    w("    pub fn resources(&self) -> Vec<ResourceRef> {")
    w("        match self {")
    for t, name in variants:
        w(f"            Typed::{name}(x) => x.resources(),")
    w("        }")
    w("    }")
    w("")
    w("    /// What the typed form does not name.")
    w("    pub fn extra(&self) -> &Extra {")
    w("        match self {")
    for t, name in variants:
        w(f"            Typed::{name}(x) => &x.extra,")
    w("        }")
    w("    }")
    w("}")
    w("")
    w("/// The argument the fxr project says the property `name` of action `id` is a function of (the seconds since the")
    w("/// particle was emitted, since its effect's node became active, the time of its emission, or nothing).")
    w("pub fn argument_of(id: i16, name: &str) -> Option<PropertyArgument> {")
    w("    use PropertyArgument::{ActiveTime, Constant0, EmissionTime, ParticleAge};")
    w("    Some(match (id, name) {")
    short = {"Constant0": "Constant0", "ParticleAge": "ParticleAge", "ActiveTime": "ActiveTime", "EmissionTime": "EmissionTime"}
    for t, n, arg in arguments:
        if arg not in short:
            raise SystemExit(f"{t} {n}: argument {arg}")
        w(f'        ({t}, "{n}") => {short[arg]},')
    w("        _ => return None,")
    w("    })")
    w("}")
    w("")
    w("/// The parameters named here and not by the fxr project, whose names come from what Elden Ring's code does with")
    w("/// them: the action type, the name used here (in `params` and the struct's documentation) and the project's.")
    w("pub const TRACED_PARAMS: &[(i16, &str, &str)] = &[")
    for t, names in TRACED_NAMES.items():
        for old, (new, _doc) in names.items():
            w(f'    ({t}, "{new}", "{old}"),')
    w("];")
    w("")
    w("/// The fxr project's name of a parameter of action type `ty` named `name` here (the same name unless")
    w("/// [`TRACED_PARAMS`] renames it).")
    w("pub fn library_name(ty: i16, name: &str) -> &str {")
    w("    TRACED_PARAMS")
    w("        .iter()")
    w("        .find(|&&(t, ours, _)| t == ty && ours == name)")
    w("        .map_or(name, |&(_, _, theirs)| theirs)")
    w("}")
    w("")
    w("/// The name of every action type that occurs in an installed file, by type number.")
    w("pub const NAMES: &[(i16, &str)] = &[")
    for t, name in variants:
        w(f"    ({t}, \"{name}\"),")
    w("];")
    sys.stdout.write("\n".join(out) + "\n")


if __name__ == "__main__":
    main(sys.argv[1])
