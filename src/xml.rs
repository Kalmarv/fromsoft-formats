//! The small subset of XML a paramdef file uses: elements with double- or single-quoted attributes, text, comments, a
//! declaration, CDATA sections and the five predefined entities and numeric character references. A document type
//! declaration, a namespace-aware reading and anything else the subset does not name are refused.
use crate::{Result, invalid};

const FORMAT: &str = "XML";

/// One element: its name, attributes in file order, the text directly inside it (trimmed) and its child elements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Element {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<Element>,
}

impl Element {
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }
}

struct Parser<'a> {
    text: &'a str,
    at: usize,
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':') || b >= 0x80
}

impl<'a> Parser<'a> {
    fn fail<T>(&self, detail: impl ToString) -> Result<T> {
        Err(invalid(FORMAT, self.at, detail))
    }

    fn rest(&self) -> &'a str {
        self.text.get(self.at..).unwrap_or("")
    }

    fn starts_with(&self, s: &str) -> bool {
        self.rest().starts_with(s)
    }

    fn skip_space(&mut self) {
        let rest = self.rest();
        self.at = self
            .at
            .saturating_add(rest.len().saturating_sub(rest.trim_start().len()));
    }

    fn skip_past(&mut self, end: &str, what: &str) -> Result<()> {
        match self.rest().find(end) {
            Some(i) => {
                self.at = self.at.saturating_add(i).saturating_add(end.len());
                Ok(())
            }
            None => self.fail(format!("{what} is not terminated")),
        }
    }

    /// Whitespace, comments and processing instructions between nodes.
    fn skip_misc(&mut self) -> Result<()> {
        loop {
            self.skip_space();
            if self.starts_with("<!--") {
                self.skip_past("-->", "a comment")?;
            } else if self.starts_with("<?") {
                self.skip_past("?>", "a declaration")?;
            } else {
                return Ok(());
            }
        }
    }

    fn name(&mut self) -> Result<&'a str> {
        let rest = self.rest();
        let len = rest.bytes().take_while(|&b| is_name_byte(b)).count();
        if len == 0 {
            return self.fail("a name was expected");
        }
        let name = rest.get(..len).unwrap_or("");
        self.at = self.at.saturating_add(len);
        Ok(name)
    }

    /// `raw` with its entity and character references replaced.
    fn unescape(&self, raw: &str) -> Result<String> {
        let mut out = String::with_capacity(raw.len());
        let mut rest = raw;
        while let Some(i) = rest.find('&') {
            out.push_str(rest.get(..i).unwrap_or(""));
            let after = rest.get(i.saturating_add(1)..).unwrap_or("");
            let Some(end) = after.find(';') else {
                return self.fail("an entity reference is not terminated");
            };
            let entity = after.get(..end).unwrap_or("");
            match entity {
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "amp" => out.push('&'),
                "quot" => out.push('"'),
                "apos" => out.push('\''),
                _ => {
                    let number = entity
                        .strip_prefix('#')
                        .and_then(|n| match n.strip_prefix(['x', 'X']) {
                            Some(hex) => u32::from_str_radix(hex, 16).ok(),
                            None => n.parse::<u32>().ok(),
                        });
                    match number.and_then(char::from_u32) {
                        Some(c) => out.push(c),
                        None => return self.fail(format!("unknown entity &{entity};")),
                    }
                }
            }
            rest = after.get(end.saturating_add(1)..).unwrap_or("");
        }
        out.push_str(rest);
        Ok(out)
    }

    fn element(&mut self, depth: usize) -> Result<Element> {
        if depth > 64 {
            return self.fail("elements nest deeper than 64 levels");
        }
        if !self.starts_with("<") {
            return self.fail("an element was expected");
        }
        self.at = self.at.saturating_add(1);
        let name = self.name()?.to_owned();
        let mut attributes: Vec<(String, String)> = Vec::new();
        loop {
            self.skip_space();
            if self.starts_with("/>") {
                self.at = self.at.saturating_add(2);
                return Ok(Element {
                    name,
                    attributes,
                    text: String::new(),
                    children: Vec::new(),
                });
            }
            if self.starts_with(">") {
                self.at = self.at.saturating_add(1);
                break;
            }
            let key = self.name()?.to_owned();
            self.skip_space();
            if !self.starts_with("=") {
                return self.fail(format!("attribute {key} has no value"));
            }
            self.at = self.at.saturating_add(1);
            self.skip_space();
            let quote = match self.rest().chars().next() {
                Some(q @ ('"' | '\'')) => q,
                _ => return self.fail(format!("attribute {key} is not quoted")),
            };
            self.at = self.at.saturating_add(1);
            let Some(len) = self.rest().find(quote) else {
                return self.fail(format!("attribute {key} is not terminated"));
            };
            let raw = self.rest().get(..len).unwrap_or("");
            if raw.contains('<') {
                return self.fail(format!("attribute {key} holds `<`"));
            }
            let value = self.unescape(raw)?;
            self.at = self.at.saturating_add(len).saturating_add(1);
            if attributes.iter().any(|(k, _)| *k == key) {
                return self.fail(format!("attribute {key} is repeated"));
            }
            attributes.push((key, value));
        }
        let mut text = String::new();
        let mut children = Vec::new();
        loop {
            let rest = self.rest();
            let Some(i) = rest.find('<') else {
                return self.fail(format!("element {name} is not closed"));
            };
            text.push_str(&self.unescape(rest.get(..i).unwrap_or(""))?);
            self.at = self.at.saturating_add(i);
            if self.starts_with("<!--") {
                self.skip_past("-->", "a comment")?;
            } else if self.starts_with("<![CDATA[") {
                self.at = self.at.saturating_add(9);
                let body = self.rest();
                let Some(end) = body.find("]]>") else {
                    return self.fail("a CDATA section is not terminated");
                };
                text.push_str(body.get(..end).unwrap_or(""));
                self.at = self.at.saturating_add(end).saturating_add(3);
            } else if self.starts_with("<?") {
                self.skip_past("?>", "a processing instruction")?;
            } else if self.starts_with("<!") {
                return self.fail("a declaration inside an element is not read");
            } else if self.starts_with("</") {
                self.at = self.at.saturating_add(2);
                let close = self.name()?;
                if close != name {
                    return self.fail(format!("</{close}> closes <{name}>"));
                }
                self.skip_space();
                if !self.starts_with(">") {
                    return self.fail("a closing tag is not terminated");
                }
                self.at = self.at.saturating_add(1);
                break;
            } else {
                children.push(self.element(depth.saturating_add(1))?);
            }
        }
        Ok(Element {
            name,
            attributes,
            text: text.trim().to_owned(),
            children,
        })
    }
}

/// The root element of the document `text`; a leading byte order mark is skipped.
pub(crate) fn parse(text: &str) -> Result<Element> {
    let mut p = Parser {
        text: text.strip_prefix('\u{feff}').unwrap_or(text),
        at: 0,
    };
    p.skip_misc()?;
    if p.starts_with("<!DOCTYPE") {
        return p.fail("a document type declaration is not read");
    }
    let root = p.element(0)?;
    p.skip_misc()?;
    if !p.rest().is_empty() {
        return p.fail("content after the root element");
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_subset() {
        let doc = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!-- c -->\n<A x=\"1\" y='a&amp;b'>\n  <B>t&lt;1&#x41;&#66;</B>\n  <C/>\n  <D><![CDATA[<z>]]></D>\n</A>\n";
        let a = parse(doc).unwrap();
        assert_eq!(a.name, "A");
        assert_eq!(a.attribute("x"), Some("1"));
        assert_eq!(a.attribute("y"), Some("a&b"));
        assert_eq!(a.child("B").unwrap().text, "t<1AB");
        assert!(a.child("C").unwrap().children.is_empty());
        assert_eq!(a.child("D").unwrap().text, "<z>");
    }

    #[test]
    fn refuses_what_it_does_not_read() {
        for bad in [
            "",
            "<A>",
            "<A></B>",
            "<A x=1/>",
            "<A x=\"1\" x=\"2\"/>",
            "<A>&nope;</A>",
            "<A/><B/>",
            "<!DOCTYPE A><A/>",
            "<A x=\"<\"/>",
            "<A><!x></A>",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }
}
