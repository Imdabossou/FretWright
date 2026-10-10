//! Pre-build processing of Rocksmith 2014 arrangements: automatic phrase and
//! section generation, and the fixes DLC Builder applies before conversion
//! (anchors at phrase starts, link-next repairs, crowd events, hand-shape
//! lengths, custom events, ...).
//!
//! Port of Rocksmith2014.NET's `Rocksmith2014.XML.Processing` (MIT, Tapio
//! Malmberg); its output was checked against the original during the port.

pub mod fixes;
pub mod phrases;
mod util;

use fw_xml::InstrumentalArrangement;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("phrase generation failed: {0}")]
    PhraseGeneration(String),
    #[error("unable to parse the value of the 'mover' phrase at {0}")]
    PhraseMover(String),
    #[error("custom event error: {0}")]
    CustomEvent(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Generates phrases when the arrangement has (almost) none, as a build does.
pub fn generate_phrases_if_needed(arr: &mut InstrumentalArrangement) -> Result<()> {
    if arr.phrase_iterations.len() <= 3 && arr.sections.len() <= 3 {
        phrases::generate(arr)?;
    }
    Ok(())
}

/// Applies every improvement, in the order DLC Builder uses.
pub fn apply_all(arr: &mut InstrumentalArrangement) -> Result<()> {
    fixes::validate_phrase_names(arr);
    fixes::add_ignores(arr);
    fixes::fix_link_nexts(arr);
    fixes::remove_overlapping_bend_values(arr);
    fixes::remove_muted_notes_from_chords(arr);
    // before the phrase start anchors are fixed
    fixes::move_anchors(arr);
    fixes::move_phrases(arr)?;
    fixes::eof_fixes(arr);
    fixes::remove_unnecessary_notes(arr);
    // after the phrase start anchors are fixed
    fixes::remove_redundant_anchors(arr);
    fixes::add_crowd_events(arr);
    fixes::process_chord_names(arr);
    fixes::process_custom_events(arr)?;
    fixes::remove_extra_beats(arr);
    fixes::lengthen_hand_shapes(arr);
    fixes::shorten_hand_shapes(arr);
    Ok(())
}

/// The minimum set of fixes needed for a valid build.
pub fn apply_minimum(arr: &mut InstrumentalArrangement) {
    fixes::validate_phrase_names(arr);
    fixes::add_ignores(arr);
    fixes::remove_overlapping_bend_values(arr);
    fixes::remove_muted_notes_from_chords(arr);
    fixes::fix_chord_notes(arr);
    fixes::fix_phrase_start_anchors(arr);
}
