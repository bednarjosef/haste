//! The multiplayer server list: `servers.dat` in the game directory.
//!
//! Vanilla 26.3 stores the list as an uncompressed NBT file whose root compound
//! holds a list of `servers` compounds, each with `name` and `ip` (strings),
//! an optional `icon` (base64 PNG) and `acceptTextures` (byte: 1 accept,
//! 0 deny, absent = prompt), plus `hidden` (byte) for direct-connect entries.
//! The NBT codec comes from `haste-protocol`; the layout (minecraft.wiki
//! "servers.dat format") is read from observation of the game, not from
//! decompiled code.
//!
//! Unknown tags in a file are preserved on save: mods add their own fields to
//! server data, and rewriting the file must not drop them.
//!
//! A `servers.dat` that cannot be parsed is logged and discarded, like vanilla's
//! `ServerList.load`, so a broken file never keeps the multiplayer screen from
//! opening.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use haste_protocol::nbt::deserializer::NbtReadHelperJava;
use haste_protocol::nbt::{Nbt, NbtCompound, NbtTag};

/// One saved server, mirroring vanilla's `ServerData` fields we model.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerEntry {
    /// Display name shown in the multiplayer screen. Empty for direct connect.
    pub name: String,
    /// Host and port, `"host:port"` (vanilla's `ip`; port may be absent).
    pub address: String,
    /// Decoded server icon (PNG bytes), if the server sent one.
    pub icon: Option<Vec<u8>>,
    /// Whether server resource packs are accepted. `None` = ask on join.
    pub accept_textures: Option<bool>,
    /// Direct-connect entry kept out of vanilla's normal listing.
    pub hidden: bool,
    /// Tags found in the file that this struct does not model (mods add their
    /// own). Written back verbatim by [`ServerList::save`].
    pub extra: NbtCompound,
}

impl ServerEntry {
    /// An entry with only the required fields set.
    #[must_use]
    pub fn new(name: impl Into<String>, address: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            address: address.into(),
            icon: None,
            accept_textures: None,
            hidden: false,
            extra: NbtCompound::new(),
        }
    }

    /// Reads one entry out of a `servers` list compound, keeping unknown tags in
    /// `extra`. Missing `name`/`ip` become empty strings (vanilla's direct-connect
    /// entries ship an empty name).
    fn from_compound(mut compound: NbtCompound) -> Self {
        let name = take_string(&mut compound, "name").unwrap_or_default();
        let address = take_string(&mut compound, "ip").unwrap_or_default();
        let hidden = take_byte(&mut compound, "hidden").is_some_and(|value| value != 0);
        let accept_textures = take_byte(&mut compound, "acceptTextures").map(|value| value != 0);
        let mut icon = None;
        match compound.child_tags.remove("icon") {
            Some(NbtTag::String(encoded)) => match BASE64.decode(encoded.as_bytes()) {
                Ok(decoded) => icon = Some(decoded),
                // Undecodable icon: keep the original string so a save cannot lose it.
                Err(_) => compound.put("icon", NbtTag::String(encoded)),
            },
            Some(other) => compound.put("icon", other),
            None => {}
        }
        Self {
            name,
            address,
            icon,
            accept_textures,
            hidden,
            extra: compound,
        }
    }

    /// The `servers` list compound for this entry, known tags overwriting
    /// whatever `extra` holds.
    fn to_compound(&self) -> NbtCompound {
        let mut compound = self.extra.clone();
        compound.put_string("ip", self.address.clone());
        compound.put_string("name", self.name.clone());
        if let Some(icon) = &self.icon {
            compound.put_string("icon", BASE64.encode(icon));
        }
        if let Some(accept) = self.accept_textures {
            compound.put("acceptTextures", NbtTag::Byte(i8::from(accept)));
        }
        if self.hidden {
            compound.put("hidden", NbtTag::Byte(1));
        }
        compound
    }
}

/// Removes and returns the named string tag.
fn take_string(compound: &mut NbtCompound, name: &str) -> Option<String> {
    match compound.child_tags.remove(name)? {
        NbtTag::String(value) => Some(value.into_string()),
        other => {
            compound.put(name, other);
            None
        }
    }
}

/// Removes and returns the named byte tag.
fn take_byte(compound: &mut NbtCompound, name: &str) -> Option<i8> {
    match compound.child_tags.remove(name)? {
        NbtTag::Byte(value) => Some(value),
        other => {
            compound.put(name, other);
            None
        }
    }
}

/// The saved server list: visible entries first, then the hidden ones (vanilla's
/// `ServerList` keeps the direct-connect entries, `hidden = true`, in a second
/// list, [`Self::entries`] returns only the visible ones, and [`Self::save`]
/// writes visible then hidden).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerList {
    /// Entries shown in the multiplayer screen, in list order.
    entries: Vec<ServerEntry>,
    /// Direct-connect entries kept out of that listing, newest first.
    hidden: Vec<ServerEntry>,
    /// Unknown root-level tags, preserved on save.
    extra: NbtCompound,
}

impl ServerList {
    /// An empty list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads `servers.dat` from `path`. A missing file is an empty list, and so
    /// is a file that does not parse: vanilla's `ServerList.load` logs the
    /// failure and keeps the list it has, and the multiplayer screen has to open
    /// no matter what is on disk. Only I/O errors (a permission problem, a
    /// directory in the way) are returned.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", path.display()));
            }
        };
        let mut reader = NbtReadHelperJava::new(std::io::Cursor::new(bytes));
        let document = match Nbt::read(&mut reader) {
            Ok(document) => document,
            Err(error) => {
                tracing::warn!(
                    "ignoring unreadable server list {}: {error}",
                    path.display()
                );
                return Ok(Self::new());
            }
        };

        let mut extra = NbtCompound::new();
        let mut entries = Vec::new();
        let mut hidden = Vec::new();
        for (name, tag) in document.root_tag.child_tags {
            if name.as_ref() == "servers" {
                if let NbtTag::List(list) = tag {
                    for item in list {
                        if let NbtTag::Compound(compound) = item {
                            let entry = ServerEntry::from_compound(compound);
                            if entry.hidden {
                                hidden.push(entry);
                            } else {
                                entries.push(entry);
                            }
                        }
                    }
                }
            } else {
                extra.put(&name, tag);
            }
        }
        Ok(Self {
            entries,
            hidden,
            extra,
        })
    }

    /// Writes the list to `path` as uncompressed NBT, via a temp file in the
    /// same directory renamed over `path` so a crash mid-write cannot truncate
    /// the saved list.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let mut root = self.extra.clone();
        let servers = self
            .entries
            .iter()
            .chain(self.hidden.iter())
            .map(|entry| NbtTag::Compound(entry.to_compound()))
            .collect();
        root.put_list("servers", servers);
        let document = Nbt::new(String::new(), root);

        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
        if let Some(parent) = parent {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let directory = parent.unwrap_or_else(|| Path::new("."));
        let mut temp = tempfile::Builder::new()
            .prefix("servers.dat")
            .suffix(".tmp")
            .tempfile_in(directory)
            .with_context(|| format!("temp file in {}", directory.display()))?;
        temp.write_all(&document.write())
            .context("writing servers.dat")?;
        temp.flush().context("flushing servers.dat")?;
        temp.persist(path)
            .map_err(|error| error.error)
            .with_context(|| format!("renaming into {}", path.display()))?;
        Ok(())
    }

    /// The visible entries, in list order.
    #[must_use]
    pub fn entries(&self) -> &[ServerEntry] {
        &self.entries
    }

    /// The hidden entries (direct-connect ones), what [`Self::save`] writes
    /// after the visible list. They are not shown by the multiplayer screen.
    #[must_use]
    pub fn hidden_entries(&self) -> &[ServerEntry] {
        &self.hidden
    }

    /// Appends an entry: a hidden one to the front of the hidden ones, a visible
    /// one to the end of the visible list, as vanilla's `add` does.
    pub fn add(&mut self, entry: ServerEntry) {
        if entry.hidden {
            self.hidden.insert(0, entry);
        } else {
            self.entries.push(entry);
        }
    }

    /// Removes the visible entry at `index`, returning it if it was there.
    pub fn remove(&mut self, index: usize) -> Option<ServerEntry> {
        if index < self.entries.len() {
            Some(self.entries.remove(index))
        } else {
            None
        }
    }

    /// Moves the visible entry at `index` one slot up; returns whether it moved.
    pub fn move_up(&mut self, index: usize) -> bool {
        if index == 0 || index >= self.entries.len() {
            return false;
        }
        self.entries.swap(index, index - 1);
        true
    }

    /// Moves the visible entry at `index` one slot down; returns whether it moved.
    pub fn move_down(&mut self, index: usize) -> bool {
        if index + 1 >= self.entries.len() {
            return false;
        }
        self.entries.swap(index, index + 1);
        true
    }
}

/// `servers.dat` next to `config.toml` in the platform config dir.
#[must_use]
pub fn path() -> Option<PathBuf> {
    dir().map(|dir| dir.join("servers.dat"))
}

fn dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "haste").map(|dirs| dirs.config_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_BYTES: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

    fn temp_path(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        (dir, path)
    }

    #[test]
    fn roundtrips_three_entries() {
        let (dir, path) = temp_path("servers.dat");
        let _keep_alive = &dir;
        let mut list = ServerList::new();
        list.add(ServerEntry::new("alpha", "alpha.example.com:25565"));
        list.add(ServerEntry::new("beta", "beta.example.com"));
        let mut hidden = ServerEntry::new("", "localhost");
        hidden.hidden = true;
        list.add(hidden);

        list.save(&path).unwrap();
        let loaded = ServerList::load(&path).unwrap();

        assert_eq!(loaded.entries(), list.entries());
        assert_eq!(loaded.hidden_entries(), list.hidden_entries());
        assert_eq!(loaded.entries().len(), 2);
        assert_eq!(loaded.entries()[0].name, "alpha");
        assert_eq!(loaded.entries()[0].address, "alpha.example.com:25565");
        assert_eq!(loaded.entries()[1].name, "beta");
        assert!(!loaded.entries()[0].hidden);
        assert_eq!(loaded.hidden_entries().len(), 1);
        assert!(loaded.hidden_entries()[0].hidden);
        assert!(loaded.hidden_entries()[0].name.is_empty());
    }

    #[test]
    fn adds_hidden_entries_at_the_front_of_the_hidden_list() {
        let mut list = ServerList::new();
        let mut first_hidden = ServerEntry::new("", "first.example.com");
        first_hidden.hidden = true;
        let mut second_hidden = ServerEntry::new("", "second.example.com");
        second_hidden.hidden = true;
        list.add(first_hidden);
        list.add(second_hidden);
        list.add(ServerEntry::new("visible", "visible.example.com"));

        assert_eq!(list.entries().len(), 1);
        assert_eq!(list.entries()[0].name, "visible");
        assert_eq!(
            list.hidden_entries()
                .iter()
                .map(|entry| entry.address.as_str())
                .collect::<Vec<_>>(),
            vec!["second.example.com", "first.example.com"],
            "hidden entries are newest first, and never appear in entries()"
        );
    }

    #[test]
    fn saves_visible_entries_before_hidden_ones() {
        let (dir, path) = temp_path("servers.dat");
        let _keep_alive = &dir;
        let mut list = ServerList::new();
        list.add(ServerEntry::new("first", "first.example.com"));
        list.add(ServerEntry::new("second", "second.example.com"));
        let mut hidden = ServerEntry::new("", "direct.example.com");
        hidden.hidden = true;
        list.add(hidden);

        list.save(&path).unwrap();

        // Read the file's `servers` list straight through the NBT codec to check
        // the on-disk order, not just what load() reconstructs from it.
        let bytes = std::fs::read(&path).unwrap();
        let mut reader = NbtReadHelperJava::new(std::io::Cursor::new(bytes));
        let document = Nbt::read(&mut reader).unwrap();
        let list_tag = document
            .root_tag
            .get("servers")
            .and_then(|tag| tag.extract_list())
            .unwrap();
        let written: Vec<String> = list_tag
            .iter()
            .filter_map(|entry| {
                entry
                    .extract_compound()?
                    .get_string("ip")
                    .map(str::to_owned)
            })
            .collect();
        assert_eq!(
            written,
            vec![
                "first.example.com",
                "second.example.com",
                "direct.example.com"
            ]
        );
    }

    #[test]
    fn unparseable_file_yields_an_empty_list() {
        let (dir, path) = temp_path("servers.dat");
        let _keep_alive = &dir;
        std::fs::write(&path, b"this is not nbt at all \xff\xff").unwrap();
        let list = ServerList::load(&path).unwrap();
        assert!(list.entries().is_empty());
        assert!(list.hidden_entries().is_empty());

        // A truncated compound: right root tag and name, cut off inside it.
        std::fs::write(&path, [0x0a, 0x00, 0x00, b'x']).unwrap();
        let list = ServerList::load(&path).unwrap();
        assert!(list.entries().is_empty());
        assert!(list.hidden_entries().is_empty());
    }

    #[test]
    fn io_errors_are_still_reported() {
        let (dir, path) = temp_path("servers.dat");
        let _keep_alive = &dir;
        std::fs::create_dir(&path).unwrap();
        assert!(
            ServerList::load(&path).is_err(),
            "an unreadable path is an error, not a silently empty list"
        );
    }

    #[test]
    fn reorders_and_removes_entries() {
        let mut list = ServerList::new();
        for name in ["a", "b", "c"] {
            list.add(ServerEntry::new(name, format!("{name}.example.com")));
        }
        assert!(!list.move_up(0), "the first entry cannot move up");
        assert!(!list.move_down(2), "the last entry cannot move down");

        assert!(list.move_down(0));
        assert_eq!(
            list.entries()
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a", "c"]
        );
        assert!(list.move_up(2));
        assert_eq!(
            list.entries()
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "c", "a"]
        );

        let removed = list.remove(1).unwrap();
        assert_eq!(removed.name, "c");
        assert!(list.remove(9).is_none());
        assert_eq!(list.entries().len(), 2);
    }

    #[test]
    fn missing_file_is_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope").join("servers.dat");
        let list = ServerList::load(&path).unwrap();
        assert!(list.entries().is_empty());
        assert!(list.hidden_entries().is_empty());
    }

    #[test]
    fn roundtrips_icon_and_accept_textures() {
        let (dir, path) = temp_path("servers.dat");
        let _keep_alive = &dir;
        let mut list = ServerList::new();
        let mut entry = ServerEntry::new("iconic", "iconic.example.com");
        entry.icon = Some(PNG_BYTES.to_vec());
        entry.accept_textures = Some(true);
        list.add(entry);

        list.save(&path).unwrap();
        let loaded = ServerList::load(&path).unwrap();

        let entry = &loaded.entries()[0];
        assert_eq!(entry.icon.as_deref(), Some(PNG_BYTES));
        assert_eq!(entry.icon.as_ref().unwrap().len(), PNG_BYTES.len());
        assert_eq!(entry.accept_textures, Some(true));

        // Deny (byte 0) survives the round trip too.
        let mut denied = ServerList::new();
        let mut denied_entry = ServerEntry::new("denier", "denier.example.com");
        denied_entry.accept_textures = Some(false);
        denied.add(denied_entry);
        denied.save(&path).unwrap();
        let denied_loaded = ServerList::load(&path).unwrap();
        assert_eq!(denied_loaded.entries()[0].accept_textures, Some(false));
    }

    #[test]
    fn preserves_unknown_tags() {
        let (dir, path) = temp_path("servers.dat");
        let _keep_alive = &dir;
        let mut list = ServerList::new();
        let mut entry = ServerEntry::new("modded", "modded.example.com");
        entry.extra.put_string("someModField", "keep me".into());
        entry.extra.put_int("modPriority", 7);
        list.add(entry);
        list.extra.put_string("rootLevelTag", "also keep".into());

        list.save(&path).unwrap();
        let loaded = ServerList::load(&path).unwrap();

        let entry = &loaded.entries()[0];
        assert_eq!(
            entry.extra.get_string("someModField"),
            Some("keep me"),
            "unknown entry tags must survive a save"
        );
        assert_eq!(entry.extra.get_int("modPriority"), Some(7));
        assert_eq!(loaded.extra.get_string("rootLevelTag"), Some("also keep"));
    }

    #[test]
    fn loads_hand_built_nbt_sample() {
        // Root compound named "" { servers: [ {...}, {...} ] }
        let mut bytes = vec![0x0a];
        push_string("", &mut bytes);
        bytes.push(0x09); // TAG_List
        push_string("servers", &mut bytes);
        bytes.push(0x0a); // element type: compound
        bytes.extend_from_slice(&2i32.to_be_bytes());
        // First entry.
        push_named_string("ip", "localhost:25565", &mut bytes);
        push_named_string("name", "local", &mut bytes);
        bytes.extend_from_slice(&[0x01]); // TAG_Byte
        push_string("acceptTextures", &mut bytes);
        bytes.push(0); // 0 = deny
        bytes.push(0x08); // TAG_String
        push_string("icon", &mut bytes);
        push_string(&BASE64.encode(PNG_BYTES), &mut bytes);
        bytes.push(0x00); // TAG_End of the entry compound
                          // Second entry, minimal.
        push_named_string("ip", "second.example.com", &mut bytes);
        bytes.push(0x00); // end of the second entry compound
        bytes.push(0x00); // end of the root compound

        let (dir, path) = temp_path("servers.dat");
        let _keep_alive = &dir;
        std::fs::write(&path, &bytes).unwrap();
        let list = ServerList::load(&path).unwrap();

        assert_eq!(list.entries().len(), 2);
        assert_eq!(list.entries()[0].name, "local");
        assert_eq!(list.entries()[0].address, "localhost:25565");
        assert_eq!(list.entries()[0].accept_textures, Some(false));
        assert_eq!(list.entries()[0].icon.as_deref(), Some(PNG_BYTES));
        assert_eq!(list.entries()[1].name, "");
        assert_eq!(list.entries()[1].address, "second.example.com");
        assert_eq!(list.entries()[1].accept_textures, None);
        assert_eq!(list.entries()[1].icon, None);
        assert!(!list.entries()[1].hidden);
    }

    /// A TAG_String payload with its u16 length prefix.
    fn push_string(value: &str, out: &mut Vec<u8>) {
        let len = u16::try_from(value.len()).unwrap();
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(value.as_bytes());
    }

    /// A named TAG_String: id, then name, then payload.
    fn push_named_string(name: &str, value: &str, out: &mut Vec<u8>) {
        out.push(0x08); // TAG_String
        push_string(name, out);
        push_string(value, out);
    }
}
