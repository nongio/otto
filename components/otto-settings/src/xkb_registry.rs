//! The keyboard layouts, variants and layout-switch options this machine's
//! XKB data knows about.
//!
//! Read from the rules registry xkeyboard-config installs as
//! `rules/evdev.xml` — the same file `xkbcli list` and every desktop's layout
//! picker read. The compositor compiles its keymap with the `evdev` rules, so
//! anything listed here is a name it accepts.
//!
//! The file is parsed by hand rather than through an XML crate or
//! libxkbregistry: only a handful of elements matter, the format is flat and
//! machine-written, and the whole thing is read once per run.

use std::path::PathBuf;
use std::sync::OnceLock;

/// One layout variant: its XKB name and the description the registry gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub name: String,
    pub description: String,
}

/// One layout and the variants it offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub name: String,
    pub description: String,
    pub variants: Vec<Variant>,
}

/// What the registry lists: the layouts, sorted by description, and the
/// options in the `grp` group — the key combinations that switch layout.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Registry {
    pub layouts: Vec<Layout>,
    pub switch_options: Vec<Variant>,
}

impl Registry {
    pub fn layout(&self, name: &str) -> Option<&Layout> {
        self.layouts.iter().find(|layout| layout.name == name)
    }

    /// The description of `layout`'s `variant`, if the registry has one.
    pub fn variant_description(&self, layout: &str, variant: &str) -> Option<&str> {
        self.layout(layout)?
            .variants
            .iter()
            .find(|v| v.name == variant)
            .map(|v| v.description.as_str())
    }

    pub fn switch_description(&self, option: &str) -> Option<&str> {
        self.switch_options
            .iter()
            .find(|o| o.name == option)
            .map(|o| o.description.as_str())
    }
}

/// The registry, read on first use. Empty when no XKB data is found, in which
/// case the pane falls back to showing the configured names as they are.
pub fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        rules_paths()
            .into_iter()
            .find_map(|path| std::fs::read_to_string(path).ok())
            .map(|xml| parse(&xml))
            .unwrap_or_default()
    })
}

/// Where `evdev.xml` may be, in the order xkbcommon itself looks: an explicit
/// `XKB_CONFIG_ROOT` first, then the usual prefixes, then NixOS's profile.
fn rules_paths() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = std::env::var_os("XKB_CONFIG_ROOT")
        .map(PathBuf::from)
        .into_iter()
        .collect();
    roots.extend(
        [
            "/usr/share/X11/xkb",
            "/usr/local/share/X11/xkb",
            "/run/current-system/sw/share/X11/xkb",
        ]
        .map(PathBuf::from),
    );
    roots
        .into_iter()
        .map(|root| root.join("rules/evdev.xml"))
        .collect()
}

/// One step through the document.
#[derive(Debug, PartialEq, Eq)]
enum Event<'a> {
    Open(&'a str),
    Close(&'a str),
    Text(&'a str),
}

/// Split `xml` into element opens, closes and the text between them.
///
/// Declarations, comments and the doctype are skipped; attributes are dropped,
/// since the registry keeps everything this module reads in element text.
fn events(xml: &str) -> Vec<Event<'_>> {
    let mut out = Vec::new();
    let mut rest = xml;
    while !rest.is_empty() {
        let Some(start) = rest.find('<') else {
            out.push(Event::Text(rest));
            break;
        };
        if start > 0 {
            out.push(Event::Text(&rest[..start]));
        }
        rest = &rest[start..];
        let end_marker = if rest.starts_with("<!--") {
            "-->"
        } else if rest.starts_with("<![CDATA[") {
            "]]>"
        } else {
            ">"
        };
        let Some(end) = rest.find(end_marker) else {
            break;
        };
        let tag = &rest[1..end];
        rest = &rest[end + end_marker.len()..];

        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            out.push(Event::Close(name.trim()));
            continue;
        }
        let self_closing = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let name = tag.split_whitespace().next().unwrap_or_default();
        out.push(Event::Open(name));
        if self_closing {
            out.push(Event::Close(name));
        }
    }
    out
}

/// Resolve the five predefined entities and numeric character references.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';') else {
            break;
        };
        let entity = &rest[1..semi];
        let resolved = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match resolved {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Read the layouts and the layout-switch options out of a rules registry.
fn parse(xml: &str) -> Registry {
    let mut registry = Registry::default();
    let mut stack: Vec<&str> = Vec::new();
    let mut text = String::new();
    // The `configItem` being read: its name and description.
    let mut name = String::new();
    let mut description = String::new();
    // The option group currently open — only `grp` is kept.
    let mut group = String::new();

    for event in events(xml) {
        match event {
            Event::Open(tag) => {
                if tag == "configItem" {
                    name.clear();
                    description.clear();
                }
                stack.push(tag);
                text.clear();
            }
            Event::Text(chunk) => text.push_str(chunk),
            Event::Close(tag) => {
                stack.pop();
                let parent = stack.last().copied();
                match (tag, parent) {
                    ("name", Some("configItem")) => name = unescape(text.trim()),
                    ("description", Some("configItem")) => description = unescape(text.trim()),
                    ("configItem", Some("layout")) => registry.layouts.push(Layout {
                        name: std::mem::take(&mut name),
                        description: std::mem::take(&mut description),
                        variants: Vec::new(),
                    }),
                    ("configItem", Some("variant")) => {
                        if let Some(layout) = registry.layouts.last_mut() {
                            layout.variants.push(Variant {
                                name: std::mem::take(&mut name),
                                description: std::mem::take(&mut description),
                            });
                        }
                    }
                    ("configItem", Some("group")) => group = std::mem::take(&mut name),
                    ("configItem", Some("option")) if group == "grp" => {
                        registry.switch_options.push(Variant {
                            name: std::mem::take(&mut name),
                            description: std::mem::take(&mut description),
                        })
                    }
                    _ => {}
                }
                text.clear();
            }
        }
    }

    registry.layouts.sort_by(|a, b| {
        a.description
            .to_lowercase()
            .cmp(&b.description.to_lowercase())
    });
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE xkbConfigRegistry SYSTEM "xkb.dtd">
<xkbConfigRegistry version="1.1">
  <modelList>
    <model><configItem><name>pc105</name><description>Generic 105-key PC</description></configItem></model>
  </modelList>
  <layoutList>
    <layout>
      <configItem>
        <name>us</name>
        <shortDescription>en</shortDescription>
        <description>English (US)</description>
        <languageList><iso639Id>eng</iso639Id></languageList>
      </configItem>
      <variantList>
        <variant>
          <configItem>
            <name>dvorak</name>
            <description>English (Dvorak)</description>
          </configItem>
        </variant>
        <variant>
          <configItem popularity="exotic">
            <name>intl</name>
            <description>English (US, intl., with dead keys)</description>
          </configItem>
        </variant>
      </variantList>
    </layout>
    <layout>
      <configItem>
        <name>it</name>
        <description>Italian</description>
      </configItem>
    </layout>
    <layout>
      <configItem>
        <name>ba</name>
        <description>Bosnian &amp; friends</description>
      </configItem>
    </layout>
  </layoutList>
  <optionList>
    <group allowMultipleSelection="true">
      <!-- The key combination used to switch between groups -->
      <configItem>
        <name>grp</name>
        <description>Switching to another layout</description>
      </configItem>
      <option>
        <configItem>
          <name>grp:alt_shift_toggle</name>
          <description>Alt+Shift</description>
        </configItem>
      </option>
    </group>
    <group allowMultipleSelection="true">
      <configItem><name>caps</name><description>Caps Lock behavior</description></configItem>
      <option><configItem><name>caps:escape</name><description>Make Caps Lock an additional Esc</description></configItem></option>
    </group>
  </optionList>
</xkbConfigRegistry>
"#;

    #[test]
    fn layouts_and_their_variants_are_read_and_sorted() {
        let registry = parse(SAMPLE);
        let names: Vec<&str> = registry.layouts.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["ba", "us", "it"]);
        assert_eq!(registry.layouts[0].description, "Bosnian & friends");

        let us = registry.layout("us").expect("us is listed");
        assert_eq!(us.description, "English (US)");
        assert_eq!(
            us.variants
                .iter()
                .map(|v| v.name.as_str())
                .collect::<Vec<_>>(),
            ["dvorak", "intl"]
        );
        assert_eq!(
            registry.variant_description("us", "intl"),
            Some("English (US, intl., with dead keys)")
        );
        assert!(registry.layout("it").expect("it").variants.is_empty());
    }

    #[test]
    fn only_the_layout_switch_group_is_kept() {
        let registry = parse(SAMPLE);
        assert_eq!(
            registry.switch_options,
            [Variant {
                name: "grp:alt_shift_toggle".into(),
                description: "Alt+Shift".into(),
            }]
        );
        // Model entries are not layouts.
        assert!(registry.layout("pc105").is_none());
    }

    #[test]
    fn entities_are_resolved() {
        assert_eq!(
            unescape("a &lt;b&gt; &#x41;&#66; &bogus; &"),
            "a <b> AB &bogus; &"
        );
    }

    /// The installed registry, where there is one, lists the layouts every
    /// xkeyboard-config ships. Skipped on a machine without XKB data.
    #[test]
    fn the_installed_registry_parses() {
        let registry = registry();
        if registry.layouts.is_empty() {
            return;
        }
        assert!(registry.layout("us").is_some());
        assert!(registry
            .switch_description("grp:alt_shift_toggle")
            .is_some());
    }
}
