//! SNG record types. Field order is the binary layout (see [`bin_struct!`]).
//!
//! Transcribed from Rocksmith2014.NET's `Rocksmith2014.SNG/Types` (MIT,
//! Tapio Malmberg), including its notes on unknown and padding fields.

use bitflags::bitflags;

use crate::bin::{Fixed, FixedStr, Pad, bin_flags, bin_struct};

bitflags! {
    /// Per-beat flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct BeatMask: i32 {
        const FIRST_BEAT_OF_MEASURE = 0b01;
        const EVEN_MEASURE = 0b10;
    }

    /// Chord template flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct ChordMask: u32 {
        const ARPEGGIO = 0b01;
        const NOP = 0b10;
    }

    /// Note and chord technique flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct NoteMask: u32 {
        const CHORD = 1 << 1;
        const OPEN = 1 << 2;
        /// Fret-hand mute for chords.
        const FRET_HAND_MUTE = 1 << 3;
        const TREMOLO = 1 << 4;
        const HARMONIC = 1 << 5;
        const PALM_MUTE = 1 << 6;
        const SLAP = 1 << 7;
        const PLUCK = 1 << 8;
        const HAMMER_ON = 1 << 9;
        const PULL_OFF = 1 << 10;
        const SLIDE = 1 << 11;
        const BEND = 1 << 12;
        const SUSTAIN = 1 << 13;
        const TAP = 1 << 14;
        const PINCH_HARMONIC = 1 << 15;
        const VIBRATO = 1 << 16;
        /// Fret-hand mute for single notes.
        const MUTE = 1 << 17;
        const IGNORE = 1 << 18;
        const LEFT_HAND = 1 << 19;
        const RIGHT_HAND = 1 << 20;
        const HIGH_DENSITY = 1 << 21;
        const UNPITCHED_SLIDE = 1 << 22;
        const SINGLE = 1 << 23;
        const CHORD_NOTES = 1 << 24;
        const DOUBLE_STOP = 1 << 25;
        const ACCENT = 1 << 26;
        const PARENT = 1 << 27;
        const CHILD = 1 << 28;
        const ARPEGGIO = 1 << 29;
        const CHORD_PANEL = 1 << 31;
    }
}

bin_flags!(BeatMask, ChordMask, NoteMask);

bin_struct! {
    pub struct Beat {
        pub time: f32,
        pub measure: i16,
        pub beat: i16,
        pub phrase_iteration: i32,
        pub mask: BeatMask,
    }
}

bin_struct! {
    pub struct Phrase {
        pub solo: i8,
        pub disparity: i8,
        pub ignore: i8,
        #[doc(hidden)]
        pub _pad: Pad<1>,
        pub max_difficulty: i32,
        pub iteration_count: i32,
        pub name: FixedStr<32>,
    }
}

bin_struct! {
    pub struct Chord {
        pub mask: ChordMask,
        pub frets: Fixed<i8, 6>,
        pub fingers: Fixed<i8, 6>,
        pub notes: Fixed<i32, 6>,
        pub name: FixedStr<32>,
    }
}

bin_struct! {
    #[derive(Copy)]
    pub struct BendValue {
        pub time: f32,
        pub step: f32,
        /// Unknown (always zero in practice).
        #[doc(hidden)]
        pub _unknown: Pad<4>,
    }
}

bin_struct! {
    #[derive(Copy)]
    pub struct BendData32 {
        pub bend_values: Fixed<BendValue, 32>,
        pub used_count: i32,
    }
}

bin_struct! {
    pub struct ChordNotes {
        pub mask: Fixed<NoteMask, 6>,
        pub bend_data: Fixed<BendData32, 6>,
        pub slide_to: Fixed<i8, 6>,
        pub slide_unpitch_to: Fixed<i8, 6>,
        pub vibrato: Fixed<i16, 6>,
    }
}

bin_struct! {
    pub struct Vocal {
        pub time: f32,
        pub note: i32,
        pub length: f32,
        pub lyric: FixedStr<48>,
    }
}

bin_struct! {
    pub struct SymbolsHeader {
        pub id: i32,
        pub unk2: i32,
        pub unk3: i32,
        pub unk4: i32,
        pub unk5: i32,
        pub unk6: i32,
        pub unk7: i32,
        /// Always 2 in practice.
        pub unk8: i32,
    }
}

bin_struct! {
    pub struct SymbolsTexture {
        pub font: FixedStr<128>,
        pub font_path_length: i32,
        #[doc(hidden)]
        pub _unknown: Pad<4>,
        pub width: i32,
        pub height: i32,
    }
}

bin_struct! {
    #[derive(Copy)]
    pub struct Rect {
        pub y_min: f32,
        pub x_min: f32,
        pub y_max: f32,
        pub x_max: f32,
    }
}

bin_struct! {
    pub struct SymbolDefinition {
        pub symbol: FixedStr<12>,
        pub outer: Rect,
        pub inner: Rect,
    }
}

bin_struct! {
    pub struct PhraseIteration {
        pub phrase_id: i32,
        pub start_time: f32,
        pub end_time: f32,
        /// Easy, medium and hard difficulty levels.
        pub difficulty: Fixed<i32, 3>,
    }
}

bin_struct! {
    /// Leftover from Rocksmith 1, unused in 2014.
    pub struct PhraseExtraInfo {
        pub phrase_id: i32,
        pub difficulty: i32,
        pub empty: i32,
        pub level_jump: i8,
        pub redundant: i16,
        #[doc(hidden)]
        pub _pad: Pad<1>,
    }
}

bin_struct! {
    pub struct NewLinkedDifficulty {
        pub level_break: i32,
        pub nld_phrases: Vec<i32>,
    }
}

bin_struct! {
    /// Leftover from Rocksmith 1, no XML equivalent.
    pub struct Action {
        pub time: f32,
        pub action_name: FixedStr<256>,
    }
}

bin_struct! {
    pub struct Event {
        pub time: f32,
        pub name: FixedStr<256>,
    }
}

bin_struct! {
    pub struct Tone {
        pub time: f32,
        pub tone_id: i32,
    }
}

bin_struct! {
    pub struct Dna {
        pub time: f32,
        pub dna_id: i32,
    }
}

/// [`Dna::dna_id`] values.
pub mod dna {
    pub const NONE: i32 = 0;
    pub const SOLO: i32 = 1;
    pub const RIFF: i32 = 2;
    pub const CHORD: i32 = 3;
}

bin_struct! {
    pub struct Section {
        pub name: FixedStr<32>,
        pub number: i32,
        pub start_time: f32,
        pub end_time: f32,
        pub start_phrase_iteration_id: i32,
        pub end_phrase_iteration_id: i32,
        pub string_mask: Fixed<i8, 36>,
    }
}

bin_struct! {
    pub struct Anchor {
        pub start_time: f32,
        pub end_time: f32,
        pub first_note_time: f32,
        pub last_note_time: f32,
        pub fret_id: i8,
        #[doc(hidden)]
        pub _pad: Pad<3>,
        pub width: i32,
        pub phrase_iteration_id: i32,
    }
}

bin_struct! {
    pub struct AnchorExtension {
        pub beat_time: f32,
        pub fret_id: i8,
        /// Unknown (always zero).
        #[doc(hidden)]
        pub _unknown: Pad<7>,
    }
}

bin_struct! {
    /// A hand shape or arpeggio.
    pub struct FingerPrint {
        pub chord_id: i32,
        pub start_time: f32,
        pub end_time: f32,
        pub first_note_time: f32,
        /// Time after which the fingering may move to the next one (-1 = end time).
        pub last_note_time: f32,
    }
}

bin_struct! {
    pub struct Note {
        pub mask: NoteMask,
        pub flags: u32,
        pub hash: u32,
        pub time: f32,
        pub string_index: i8,
        pub fret: i8,
        pub anchor_fret: i8,
        pub anchor_width: i8,
        pub chord_id: i32,
        pub chord_notes_id: i32,
        pub phrase_id: i32,
        pub phrase_iteration_id: i32,
        pub finger_print_id: Fixed<i16, 2>,
        pub next_iter_note: i16,
        pub prev_iter_note: i16,
        pub parent_prev_note: i16,
        pub slide_to: i8,
        pub slide_unpitch_to: i8,
        pub left_hand: i8,
        pub tap: i8,
        pub pick_direction: i8,
        pub slap: i8,
        pub pluck: i8,
        pub vibrato: i16,
        pub sustain: f32,
        pub max_bend: f32,
        pub bend_data: Vec<BendValue>,
    }
}

bin_struct! {
    pub struct Level {
        pub difficulty: i32,
        pub anchors: Vec<Anchor>,
        pub anchor_extensions: Vec<AnchorExtension>,
        pub hand_shapes: Vec<FingerPrint>,
        pub arpeggios: Vec<FingerPrint>,
        pub notes: Vec<Note>,
        pub average_notes_per_iteration: Vec<f32>,
        pub notes_in_phrase_iterations_excl_ignored: Vec<i32>,
        pub notes_in_phrase_iterations_all: Vec<i32>,
    }
}
