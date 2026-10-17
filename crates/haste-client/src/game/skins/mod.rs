//! Player skins (vanilla `SkinManager` + `SkinTextureDownloader` + the `TextureManager`
//! registration): each profile's `textures` property is unpacked when its player-info entry
//! arrives (game thread), its skin and cape download off the game and render threads (the
//! tokio runtime; disk cache first), and the render thread registers finished textures with the
//! renderer and hands the resolved [`PlayerSkin`] to the entity store. Until then, and when
//! anything fails, the player wears its default skin (`SkinManager.createLookup`): nothing
//! ever waits on a download.
//!
//! The disk cache lives in the game directory (`<game dir>/skins`, vanilla's
//! `<assets>/skins` layout: `<first two hex digits>/<sha1>`), outside the repo.

pub mod download;
pub mod textures;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use ahash::AHashMap;
use haste_core::Identifier;
use haste_entity::skin::{PlayerModelType, PlayerSkin};
use uuid::Uuid;

use download::TextureType;
use textures::ProfileTextures;

/// One texture's load state (`TextureCache`'s future), by type and texture hash.
#[derive(Clone, Debug)]
enum TextureState {
    Loading,
    Ready(Identifier),
    Failed,
}

/// A profile waiting for its textures (`SkinManager.registerTextures`' `allOf`).
struct PendingSkin {
    uuid: Uuid,
    skin: Option<(TextureType, String)>,
    cape: Option<(TextureType, String)>,
    model: PlayerModelType,
    secure: bool,
}

/// A finished download: the texture's key and its id and image, or `None` on failure.
type Finished = (
    (TextureType, String),
    Option<(Identifier, Arc<image::RgbaImage>)>,
);

pub struct SkinManager {
    root: PathBuf,
    /// The local player's profile id (vanilla `requireSecure = !isLocalPlayer`).
    local: Uuid,
    textures: AHashMap<(TextureType, String), TextureState>,
    pending: Vec<PendingSkin>,
    tx: crossbeam_channel::Sender<Finished>,
    rx: crossbeam_channel::Receiver<Finished>,
    /// Every loaded texture, in load order, and how many the current renderer has.
    loaded: Vec<(Identifier, Arc<image::RgbaImage>)>,
    registered: usize,
}

impl SkinManager {
    /// `root` is the disk cache (`<game dir>/skins`), `local` our own profile id.
    pub fn new(root: PathBuf, local: Uuid) -> SkinManager {
        let (tx, rx) = crossbeam_channel::unbounded();
        SkinManager {
            root,
            local,
            textures: AHashMap::new(),
            pending: Vec::new(),
            tx,
            rx,
            loaded: Vec::new(),
            registered: 0,
        }
    }

    /// A player-info entry added `uuid` with these profile properties (game thread): unpacks
    /// its `textures` property and starts the downloads it needs. A profile without textures
    /// (offline mode) keeps its default skin; so does another player's unsigned property
    /// (vanilla only trusts insecure textures for the local player).
    pub fn request(&mut self, uuid: Uuid, property: Option<(&str, Option<&str>)>) {
        let Some((value, signature)) = property else {
            return;
        };
        let t = textures::unpack_textures(value, signature);
        self.request_textures(uuid, t);
    }

    fn request_textures(&mut self, uuid: Uuid, t: ProfileTextures) {
        if t.skin.is_none() && t.cape.is_none() {
            return;
        }
        if !t.signed && uuid != self.local {
            tracing::debug!("unsigned textures for {uuid}: default skin");
            return;
        }
        let model = match &t.skin {
            Some(s) => PlayerModelType::by_legacy_services_name(s.model.as_deref()),
            None => haste_entity::skin::default_skin(uuid).model,
        };
        let skin = t.skin.map(|s| self.get_or_load(TextureType::Skin, s));
        let cape = t.cape.map(|c| self.get_or_load(TextureType::Cape, c));
        self.pending.retain(|p| p.uuid != uuid);
        self.pending.push(PendingSkin {
            uuid,
            skin,
            cape,
            model,
            secure: t.signed,
        });
    }

    /// `TextureCache.getOrLoad`: one download per texture hash, shared by every profile.
    fn get_or_load(
        &mut self,
        kind: TextureType,
        t: textures::ProfileTexture,
    ) -> (TextureType, String) {
        let key = (kind, t.hash.clone());
        if self.textures.contains_key(&key) {
            return key;
        }
        let (id, file) = download::cache_location(&self.root, kind, &t.hash);
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("no runtime to download {}", t.url);
            self.textures.insert(key.clone(), TextureState::Failed);
            return key;
        };
        self.textures.insert(key.clone(), TextureState::Loading);
        let tx = self.tx.clone();
        let done = key.clone();
        rt.spawn(async move {
            let result = download::download(file, t.url.clone(), kind == TextureType::Skin).await;
            let finished = match result {
                Ok(image) => Some((id, Arc::new(image))),
                Err(e) => {
                    tracing::warn!("failed to load texture {}: {e:#}", t.url);
                    None
                }
            };
            let _ = tx.send((done, finished));
        });
        key
    }

    /// Render thread, once per frame: takes finished downloads, registers every loaded texture
    /// the renderer does not have yet (all of them again after a resource pack reload built a
    /// new renderer) and returns the profiles whose textures are all done, for the entity
    /// store (applied by the caller after releasing this manager, so no lock is held across
    /// both). Cheap when nothing changed: one channel poll, one map lookup, no allocation.
    pub fn pump(&mut self, renderer: &mut haste_render::Renderer) -> Vec<(Uuid, Arc<PlayerSkin>)> {
        let mut resolved = Vec::new();
        let mut changed = false;
        while let Ok((key, finished)) = self.rx.try_recv() {
            changed = true;
            let state = match finished {
                Some((id, image)) => {
                    self.loaded.push((id.clone(), image));
                    TextureState::Ready(id)
                }
                None => TextureState::Failed,
            };
            self.textures.insert(key, state);
        }
        if self
            .loaded
            .first()
            .is_some_and(|(id, _)| !renderer.has_registered_texture(id))
        {
            self.registered = 0;
        }
        for (id, image) in &self.loaded[self.registered..] {
            renderer.register_texture(id.clone(), image.clone());
        }
        self.registered = self.loaded.len();
        if changed {
            self.resolve(&mut resolved);
        }
        resolved
    }

    /// Resolves the pending profiles whose textures all finished (`registerTextures`'
    /// `allOf`): a profile with a failed texture keeps its default skin, like vanilla's
    /// failed future.
    fn resolve(&mut self, resolved: &mut Vec<(Uuid, Arc<PlayerSkin>)>) {
        let textures = &self.textures;
        let state = |k: &Option<(TextureType, String)>| k.as_ref().map(|k| textures.get(k));
        self.pending.retain(|p| {
            let (skin, cape) = (state(&p.skin), state(&p.cape));
            let loading =
                |s: &Option<Option<&TextureState>>| matches!(s, Some(Some(TextureState::Loading)));
            if loading(&skin) || loading(&cape) {
                return true;
            }
            let ready = |s: Option<Option<&TextureState>>| match s {
                None => Ok(None),
                Some(Some(TextureState::Ready(id))) => Ok(Some(id.clone())),
                _ => Err(()),
            };
            match (ready(skin), ready(cape)) {
                (Ok(body), Ok(cape)) => {
                    let body = body
                        .unwrap_or_else(|| haste_entity::skin::default_skin(p.uuid).body.clone());
                    let skin = PlayerSkin::new(body, cape.as_ref(), p.model, p.secure);
                    tracing::info!(
                        "skin for {}: {} ({:?}), cape {:?}",
                        p.uuid,
                        skin.body,
                        p.model,
                        skin.cape_texture().map(|c| c.to_string())
                    );
                    resolved.push((p.uuid, Arc::new(skin)));
                }
                _ => tracing::warn!("skin for {} failed: default skin", p.uuid),
            }
            false
        });
    }
}

/// A profile a `player_info_update` added: its id and its `textures` property (value and
/// signature), if it had one.
pub type ProfileRequest = (Uuid, Option<(String, Option<String>)>);

/// The profiles an update adds (`ADD_PLAYER` entries), queued for [`request_profiles`].
pub fn added_profiles(
    u: &haste_protocol::packets::play::clientbound::PlayerInfoUpdate,
    out: &mut Vec<ProfileRequest>,
) {
    for e in &u.entries {
        if let Some((_, properties)) = &e.add {
            let textures = properties
                .iter()
                .find(|p| p.name == "textures")
                .map(|p| (p.value.clone(), p.signature.clone()));
            out.push((e.uuid, textures));
        }
    }
}

/// Game thread, after the packets are applied (no other lock held): hands the queued profiles
/// to the manager.
pub fn request_profiles(skins: &parking_lot::Mutex<SkinManager>, queued: &mut Vec<ProfileRequest>) {
    if queued.is_empty() {
        return;
    }
    let mut skins = skins.lock();
    for (uuid, textures) in queued.drain(..) {
        skins.request(
            uuid,
            textures.as_ref().map(|(v, s)| (v.as_str(), s.as_deref())),
        );
    }
}

/// Render thread, once per frame: [`SkinManager::pump`], then the resolved skins into the
/// entity store (after the manager's lock is released).
pub fn pump(shared: &super::Shared, renderer: &mut haste_render::Renderer) {
    let resolved = shared.skins.lock().pump(renderer);
    if !resolved.is_empty() {
        let mut store = shared.entities.write();
        for (uuid, skin) in resolved {
            store.set_player_skin(uuid, skin);
        }
    }
}

/// The local player's current skin (its resolved one, else its default).
pub fn local_skin(shared: &super::Shared) -> Arc<PlayerSkin> {
    let local = shared.skins.lock().local;
    shared.entities.read().player_skin(local)
}

/// Our own model customisation. Haste's client information always sends every part
/// (`ClientInformation::defaults`, 0x7f), which is what the server syncs back as our
/// `Avatar.DATA_PLAYER_MODE_CUSTOMISATION`.
pub const LOCAL_MODEL_PARTS: haste_entity::avatar::ModelParts =
    haste_entity::avatar::ModelParts::ALL;

/// Dresses the local player's own model like a tracked one (`AvatarRenderer`), with its
/// cape swing from our cloak, walk distance and bob (`AbstractClientPlayer`'s
/// `ClientAvatarState`). The fall-flying lean scale is 1 while gliding (the glide's
/// 10-tick ramp-in is not tracked for the local player).
pub fn dress_local(
    inst: &mut haste_core::model::EntityInstance,
    skin: &PlayerSkin,
    view: &super::ViewState,
    partial: f32,
) {
    use haste_entity::avatar::{cape_state, dress, CapeInput, CapeState};
    let lerp = |a: f32, b: f32| a + (b - a) * partial;
    let cape = if skin.cape.is_some() {
        cape_state(&CapeInput {
            cloak: view.cloak.at(partial),
            pos: inst.pos,
            body_yaw: inst.yaw,
            fall_flying_scale: if view.pose == Some(haste_core::entity::Pose::FallFlying) {
                1.0
            } else {
                0.0
            },
            bob: lerp(view.bob_o, view.bob),
            walk_dist: lerp(view.walk_dist_o, view.walk_dist),
        })
    } else {
        CapeState::default()
    };
    dress(inst, skin, LOCAL_MODEL_PARTS, &cape);
}
