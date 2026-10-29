#!/usr/bin/env python3
"""Generates the oracle `oracles.soulstruct.elden/tae-event-classes.json`: the TAE event types soulstruct lists and the layout of
the data of each, as the second source for the community event templates (`tests/tae_template_census.rs`).

Input: soulstruct's `src/soulstruct/base/animations/tae/enums.py` (the `TAEEventType` members) and `events.py` (one `BinaryStruct`
class per event type, its fields in order; soulstruct 12b6918, GPL-3.0; the two files are parsed with `ast` and nothing is
executed or copied, only the names, ids, field widths and types are written). The classes are those of the Dark Souls
Remastered and Dark Souls III era, which soulstruct reads (`0x1000C` files); Sekiro's and Elden Ring's files are `0x1000D`.

    python -I tools/gen_soulstruct_tae_events.py --soulstruct PATH/TO/soulstruct --out tae-event-classes.json

Writes `{"source", "events": {"<id>": {"name", "class", "fields"}}}`; `fields` is null for an id with no class, else a list of
`[offset, width, type, name, pad]` in order (types: int32 uint32 int16 ushort byte sbyte bool float, or `list[byte]` and
`list[int32]` for a fixed array, width the array's whole width; `pad` true for `binary_pad` bytes). Offsets are the running sum of
the widths (`BinaryStruct` packs its fields in order).
"""
import argparse
import ast
import json
import os

WIDTH = {"int32": 4, "uint32": 4, "float": 4, "int16": 2, "ushort": 2, "sbyte": 1, "byte": 1, "bool": 1}
ROOT = "src/soulstruct/base/animations/tae/"


def parse(path):
    with open(path, encoding="utf-8") as f:
        return ast.parse(f.read())


def enum_members(tree):
    for node in tree.body:
        if isinstance(node, ast.ClassDef) and node.name == "TAEEventType":
            return {s.targets[0].id: s.value.value for s in node.body if isinstance(s, ast.Assign)}
    raise SystemExit("no TAEEventType")


def event_classes(tree, members):
    classes = {}
    for node in tree.body:
        if not isinstance(node, ast.ClassDef) or node.name == "TAEEventData":
            continue
        kind = None
        fields = []
        at = 0
        for s in node.body:
            if isinstance(s, ast.Assign) and getattr(s.targets[0], "id", "") == "event_type":
                kind = members[s.value.attr]
            elif isinstance(s, ast.AnnAssign):
                annotation = ast.unparse(s.annotation)
                value = ast.unparse(s.value) if s.value else ""
                pad = annotation == "bytes"
                if pad:
                    assert value.startswith("binary_pad("), (node.name, value)
                    width = int(value[len("binary_pad("):-1])
                elif annotation.startswith("list["):
                    assert value.startswith("binary_array("), (node.name, value)
                    width = int(value[len("binary_array("):-1]) * WIDTH[annotation[len("list["):-1]]
                else:
                    width = WIDTH[annotation]
                fields.append([at, width, annotation, s.target.id, pad])
                at += width
        assert kind is not None, node.name
        classes[kind] = (node.name, fields)
    return classes


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--soulstruct", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    members = enum_members(parse(os.path.join(args.soulstruct, ROOT + "enums.py")))
    classes = event_classes(parse(os.path.join(args.soulstruct, ROOT + "events.py")), members)
    events = {}
    for name, kind in members.items():
        cls, fields = classes.get(kind, (None, None))
        events[str(kind)] = {"name": name, "class": cls, "fields": fields}
    with open(args.out, "w", encoding="utf-8", newline="\n") as f:
        json.dump({"source": "soulstruct 12b6918 src/soulstruct/base/animations/tae/{enums,events}.py; tools/gen_soulstruct_tae_events.py",
                   "events": events}, f, indent=0, sort_keys=True)
        f.write("\n")


if __name__ == "__main__":
    main()
