//! BLF load lists (`.blf`): the text member of a divided binder that tells the packager which files go into which
//! division of the binder. Elden Ring keeps a character's animations in divisions: `c2010.anibnd.dcx` holds the skeleton and
//! the members `c2010_div00.blf`, `c2010_div01.blf`, and the animations are in the binders `c2010_div00.anibnd.dcx` and
//! `c2010_div01.anibnd.dcx` (`Binder::division_markers` names them). A list is plain text, `BLI3` on its first line:
//!
//! ```text
//! BLI3
//! #FILE_VERSION = 301
//! // comment lines (Shift-JIS)
//! $1000000000
//!     "\\INTERROOT[\w\d_]*\\chr\\c\d{4}\\hkx\\(?:a(000)_([0-9]\d{5})|a(002)_(00300[0-7]|00[0-2]\d{3}))\.hkx$",  16, 2, 1, _HKX, "($1)($2)($3)($4)"
//! $6000000
//!     "\\Model\\chr\\blf\\div_data\.txt",  16, 2, 1, _TXT, ""
//! ```
//!
//! Each command is a `$` and an entry id, and on the line after it a path pattern, three numbers, the kind of file
//! (`_HKX`, `_TXT`) and a replacement. What the pattern and the numbers mean to the packager is not known: they are kept
//! as written. In all 261 lists of Elden Ring's install (96 character binders) the file is the same three commands, the
//! first with the pattern of the animations that division holds.
use crate::{Result, invalid};

const FORMAT: &str = "BLF";

/// One command of a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    /// The entry id after the `$`: the id the first matching file takes (1,000,000,000 for the animations,
    /// 6,000,000 for `div_data.txt`, 9,999,999 for `ver_0001.txt` in every list of the install).
    pub first_entry_id: u64,
    /// The first quoted field: a path pattern in a regular-expression dialect, as written.
    pub pattern: String,
    /// The three numbers after it (16, 2, 1 in every command of the install), as written.
    pub parameters: [u32; 3],
    /// The kind of file: the word after the numbers, with its underscore (`_HKX`, `_TXT`).
    pub kind: String,
    /// The last quoted field, as written (a group listing such as `($1)($2)`, or empty).
    pub replacement: String,
}

/// A read load list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadList {
    /// The number after `#FILE_VERSION =` (301 in every list of the install).
    pub version: u32,
    /// The commands in file order.
    pub commands: Vec<Command>,
}

/// The text of `line` as characters, one per byte (the comments are Shift-JIS and nothing else is not ASCII).
fn latin1(line: &[u8]) -> String {
    line.iter().map(|b| char::from(*b)).collect()
}

/// The quoted string `s` starts with (after blanks), without its quotes, and what follows it: the string ends at the first
/// quote that is followed, after blanks, by a comma or the end of the line.
fn quoted(s: &str) -> Option<(&str, &str)> {
    let inner = s.trim_start().strip_prefix('"')?;
    let mut from = 0usize;
    loop {
        let at = from.saturating_add(inner.get(from..)?.find('"')?);
        let after = inner.get(at.saturating_add(1)..)?;
        let rest = after.trim_start();
        if rest.is_empty() || rest.starts_with(',') {
            return Some((inner.get(..at)?, after));
        }
        from = at.saturating_add(1);
    }
}

impl LoadList {
    /// Read a list. The first line must be `BLI3`, the second `#FILE_VERSION = <number>`; after them blank lines and `//`
    /// comments are skipped and every other line must be a command (`$<id>`) or the six fields that follow one. A list
    /// that breaks the shape, or a command without its fields, is refused with the byte offset of the line.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut lines = bytes.split_inclusive(|b| *b == b'\n');
        let mut offset = 0usize;
        let mut next = |offset: &mut usize| -> Option<(usize, String)> {
            let raw = lines.next()?;
            let at = *offset;
            *offset = offset.saturating_add(raw.len());
            Some((at, latin1(raw).trim().to_owned()))
        };
        match next(&mut offset) {
            Some((_, first)) if first == "BLI3" => {}
            _ => return Err(invalid(FORMAT, 0, "not a BLI3 list")),
        }
        let version = match next(&mut offset) {
            Some((at, line)) => line
                .strip_prefix("#FILE_VERSION")
                .and_then(|r| r.trim_start().strip_prefix('='))
                .and_then(|r| r.trim().parse::<u32>().ok())
                .ok_or_else(|| invalid(FORMAT, at, "no #FILE_VERSION line after BLI3"))?,
            None => return Err(invalid(FORMAT, offset, "no #FILE_VERSION line after BLI3")),
        };
        let mut commands = Vec::new();
        let mut pending: Option<u64> = None;
        while let Some((at, line)) = next(&mut offset) {
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            match pending.take() {
                None => {
                    let id = line
                        .strip_prefix('$')
                        .and_then(|d| d.parse::<u64>().ok())
                        .ok_or_else(|| invalid(FORMAT, at, format!("{line:?} is not a command")))?;
                    pending = Some(id);
                }
                Some(first_entry_id) => {
                    let field_error =
                        || invalid(FORMAT, at, format!("{line:?} is not a command's six fields"));
                    let (pattern, rest) = quoted(&line).ok_or_else(field_error)?;
                    let rest = rest.trim_start().strip_prefix(',').ok_or_else(field_error)?;
                    let mut fields = rest.splitn(5, ',');
                    let mut number = || -> Result<u32> {
                        fields
                            .next()
                            .and_then(|f| f.trim().parse().ok())
                            .ok_or_else(field_error)
                    };
                    let parameters = [number()?, number()?, number()?];
                    let kind = fields
                        .next()
                        .map(str::trim)
                        .filter(|k| k.starts_with('_'))
                        .ok_or_else(field_error)?;
                    let tail = fields.next().ok_or_else(field_error)?;
                    let (replacement, after) = quoted(tail).ok_or_else(field_error)?;
                    if !after.trim().is_empty() {
                        return Err(field_error());
                    }
                    commands.push(Command {
                        first_entry_id,
                        pattern: pattern.to_owned(),
                        parameters,
                        kind: kind.to_owned(),
                        replacement: replacement.to_owned(),
                    });
                }
            }
        }
        if pending.is_some() {
            return Err(invalid(FORMAT, offset, "a command without its fields at the end"));
        }
        Ok(Self { version, commands })
    }
}

#[cfg(test)]
mod tests {
    use super::LoadList;

    const LIST: &[u8] = b"BLI3\r\n#FILE_VERSION = 301\r\n\r\n// \x83t\x83@\x83C\x83\x8b\r\n\r\n$1000000000\r\n\t\"\\\\INTERROOT[\\w\\d_]*\\\\chr\\\\c\\d{4}\\\\hkx\\\\(?:a(000)_([0-9]\\d{5}))\\.hkx$\",\t16,\t2,\t1,\t_HKX,\t\"($1)($2)\"\r\n$6000000\r\n\t\"\\\\Model\\\\chr\\\\blf\\\\div_data\\.txt\" , 16 , 2 , 1 , _TXT     , \"\"\r\n";

    #[test]
    fn a_list_reads_its_commands() {
        let l = LoadList::parse(LIST).expect("a list");
        assert_eq!(l.version, 301);
        assert_eq!(l.commands.len(), 2);
        let c = &l.commands[0];
        assert_eq!(c.first_entry_id, 1_000_000_000);
        assert!(c.pattern.starts_with("\\\\INTERROOT") && c.pattern.ends_with("\\.hkx$"));
        assert_eq!(
            (c.parameters, c.kind.as_str(), c.replacement.as_str()),
            ([16, 2, 1], "_HKX", "($1)($2)")
        );
        let c = &l.commands[1];
        assert_eq!(
            (c.first_entry_id, c.kind.as_str(), c.replacement.as_str()),
            (6_000_000, "_TXT", "")
        );
        assert_eq!(c.parameters, [16, 2, 1]);
        // a list with no commands is fine; line feeds alone end lines too
        assert_eq!(
            LoadList::parse(b"BLI3\n#FILE_VERSION = 5\n")
                .unwrap()
                .commands
                .len(),
            0
        );
    }

    #[test]
    fn a_list_that_breaks_the_shape_is_refused_at_its_line() {
        for bad in [
            &b""[..],
            b"BLI2\n#FILE_VERSION = 301\n",
            b"BLI3\n",
            b"BLI3\n#FILE_VERSION = x\n",
            b"BLI3\n#FILE_VERSION = 301\nnonsense\n",
            b"BLI3\n#FILE_VERSION = 301\n$1\n",
            b"BLI3\n#FILE_VERSION = 301\n$1\n\"a\", 1, 2, _HKX, \"\"\n",
            b"BLI3\n#FILE_VERSION = 301\n$1\n\"a\", 1, 2, 3, HKX, \"\"\n",
            b"BLI3\n#FILE_VERSION = 301\n$1\n\"a\", 1, 2, 3, _HKX, \"\" extra\n",
            b"BLI3\n#FILE_VERSION = 301\n$1\n$2\n",
        ] {
            assert!(
                LoadList::parse(bad).is_err(),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
        let e = LoadList::parse(b"BLI3\n#FILE_VERSION = 301\n\nnonsense\n")
            .unwrap_err()
            .to_string();
        assert!(e.starts_with("BLF at 0x1a:"), "{e}");
    }
}
