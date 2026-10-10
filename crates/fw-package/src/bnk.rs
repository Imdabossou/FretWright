//! Wwise sound banks that point the game at a .wem stream (port of SoundBank.fs).

use crate::{Platform, Rng};

/// FNV-1 hash of the lowercased name, as Wwise event IDs use.
pub fn fnv_hash(s: &str) -> u32 {
    s.to_lowercase().chars().fold(2_166_136_261u32, |h, c| h.wrapping_mul(16_777_619) ^ c as u32)
}

/// Deterministic stand-in for the original's `abs (hash name)` file ID.
pub fn file_id(name: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h & 0x7fff_ffff
}

struct W(Vec<u8>);

impl W {
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i32(&mut self, v: i32) -> &mut Self {
        self.u32(v as u32)
    }
    fn i16(&mut self, v: i16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i8(&mut self, v: i8) -> &mut Self {
        self.0.push(v as u8);
        self
    }
    fn f32(&mut self, v: f32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
}

fn chunk(out: &mut W, name: &[u8; 4], data: &[u8]) {
    out.bytes(name).i32(data.len() as i32).bytes(data);
}

/// Builds `song_<name>.bnk` for an audio stream; returns (bank, file ID).
pub fn generate(name: &str, wem: &[u8], volume: f32, _platform: Platform, rng: &mut Rng) -> (Vec<u8>, u32) {
    let bank_id = rng.next_positive() as u32;
    let file_id = file_id(name);
    let sound_id = rng.next_positive() as u32;
    let preview = crate::strings::ends_with_ci(name, "_Preview");
    let prefetch = if preview { 72_000 } else { 51_200 };
    let mixer_id = 650_605_636u32;
    let action_id = rng.next_positive() as u32;
    let parent_bus = rng.next_positive() as u32;

    let mut header = W(Vec::new());
    header.u32(91).u32(bank_id).u32(0).u32(0).i32(0).i32(0).i32(0);

    let mut index = W(Vec::new());
    index.u32(file_id).i32(0).i32(prefetch as i32);

    let data = &wem[..wem.len().min(prefetch)];

    let mut sound = W(Vec::new());
    sound
        .u32(sound_id)
        .u32(262_145) // Vorbis plugin
        .u32(2) // streamed, zero latency
        .u32(file_id)
        .u32(file_id)
        .i8(0)
        .i8(0)
        .i8(0)
        .u32(parent_bus)
        .u32(65_536)
        .u32(if preview { 4_178_100_890 } else { 0 })
        .u32(mixer_id)
        .i8(0)
        .i8(0)
        .i8(0)
        .i8(3) // parameters: volume, 46, 47
        .i8(0)
        .i8(46)
        .i8(47)
        .f32(volume)
        .i32(1)
        .i32(3)
        .i8(0)
        .i8(0)
        .i8(0)
        .i8(0)
        .i8(0)
        .i8(0)
        .i8(1) // play from elapsed time
        .i8(preview as i8)
        .i8(0)
        .i16(if preview { 1 } else { 0 })
        .i8(0)
        .i8(0)
        .i8(preview as i8)
        .i8(0)
        .i32(0)
        .i16(0)
        .i32(0);

    let mixer = actor_mixer(mixer_id, sound_id);

    let mut action = W(Vec::new());
    action.u32(action_id).i8(3).i8(4).u32(sound_id).i8(0).i8(0).i8(0).i8(4).u32(bank_id);

    let mut event = W(Vec::new());
    event.u32(fnv_hash(&format!("Play_{name}"))).u32(1).u32(action_id);

    let mut hirc = W(Vec::new());
    hirc.u32(4);
    for (id, obj) in [(2i8, sound), (7, mixer), (3, action), (4, event)] {
        hirc.i8(id).i32(obj.0.len() as i32).bytes(&obj.0);
    }

    let bank_name = format!("Song_{name}");
    let mut stid = W(Vec::new());
    stid.u32(1).u32(1).u32(bank_id).i8(bank_name.len() as i8).bytes(bank_name.as_bytes());

    let mut out = W(Vec::new());
    chunk(&mut out, b"BKHD", &header.0);
    chunk(&mut out, b"DIDX", &index.0);
    chunk(&mut out, b"DATA", data);
    chunk(&mut out, b"HIRC", &hirc.0);
    chunk(&mut out, b"STID", &stid.0);
    (out.0, file_id)
}

fn actor_mixer(mixer_id: u32, sound_id: u32) -> W {
    let mut m = W(Vec::new());
    m.u32(mixer_id)
        .i8(0) // overrideParent
        .i8(0) // numFX
        .u32(2_616_261_673) // parentBusID
        .u32(0) // directParentID
        .u32(0) // unkID1
        .u32(65_792) // unkID2
        .i8(0) // priorityOverrideParent
        .i8(0) // priorityApplyDist
        .i8(0) // numParam
        .i8(0) // numRange
        .i8(0) // positionOverride
        .i8(0) // overrideGameAux
        .i8(0) // useGameAux
        .i8(0) // overrideUserAux
        .i8(0) // hasAux
        .i8(0) // virtualQueueBehavior
        .i8(0) // killNewest
        .i8(0) // useVirtualBehavior
        .i16(0) // maxNumInstance
        .i8(0) // isGlobalLimit
        .i8(0) // belowThresholdBehavior
        .i8(0) // isMaxNumInstOverrideParent
        .i8(0) // isVirtualVoiceOptOverrideParent
        .i32(0) // stateGroupList
        .i16(0) // rtpcList
        .i32(1) // numChild
        .u32(sound_id);
    m
}
