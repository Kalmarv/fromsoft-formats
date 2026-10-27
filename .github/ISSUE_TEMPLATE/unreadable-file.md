---
name: File this crate cannot read
about: A reader refused a real game file, or read it wrongly
title: "[format] error at 0xOFFSET: short description"
labels: unreadable-file
---

Please do not attach game files, archive keys, `regulation.key`, the Oodle library or path dictionaries.

**Game and version** (Sekiro or Elden Ring; the patch or build if you know it, and whether DLC is installed):

**File** (its archive path, e.g. `/map/mapstudio/m10_00_00_00.msb.dcx`, or the member's name in its binder):

**What you called** (e.g. `fromsoft_formats::msb::MapLayout::read`, on bytes from `Archive::read`):

**The full error message** (it names the format and the byte offset):

```
```

**File size in bytes, and SHA-256 of the bytes you passed to the reader:**

**A hex dump around the offset** (about 32 bytes before and 64 after; for example `xxd -s <offset - 32> -l 96 file`):

```
```

**What you expected** (for a file read wrongly rather than refused: the value you expected and the value you got):

**Crate version or commit:**
