//! Name and sort-value rules (port of StringValidator.fs, the parts packaging uses).

fn official_artist_sort(value: &str) -> Option<&'static str> {
    Some(match value.to_lowercase().as_str() {
        "a day to remember" => "A Day To Remember",
        "a flock of seagulls" => "A Flock of Seagulls",
        "b.b. king" => "BB King",
        "blink-182" => "Blink 182",
        "blue öyster cult" => "Blue Oyster Cult",
        "booker t. & the m.g.'s" => "Booker T & The MGs",
        "bobby \"blue\" bland" => "Bobby Blue Bland",
        "brooks & dunn" => "Brooks Dunn",
        "foster the people" => "Foster People",
        "grace potter & the nocturnals" => "Grace Potter and The Nocturnals",
        "hail the sun" => "Hail Sun",
        "the mamas & the papas" => "Mamas and The Papas",
        "motörhead" => "Motorhead",
        "mötley crüe" => "Motley Crue",
        "t. rex" => "T Rex",
        _ => return None,
    })
}

/// DLC keys are alphanumeric only.
pub fn dlc_key(input: &str) -> String {
    input.chars().filter(char::is_ascii_alphanumeric).collect()
}

/// Sort fields must start with an alphanumeric character.
pub fn sort_field(input: &str) -> String {
    input.trim_start_matches(|c: char| !c.is_ascii_alphanumeric()).to_string()
}

/// Removes a leading "the ", "a " or "an " (any case).
pub fn remove_articles(input: &str) -> &str {
    for a in ["the ", "a ", "an "] {
        if starts_with_ci(input, a) {
            return &input[a.len()..];
        }
    }
    input
}

pub fn artist_sort(artist: &str) -> String {
    sort_field(official_artist_sort(artist).unwrap_or_else(|| remove_articles(artist)))
}

pub fn title_sort(title: &str) -> String {
    sort_field(remove_articles(title))
}

/// A display value with its sort value (as `SortableString.Create` builds it).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sortable {
    pub value: String,
    pub sort: String,
}

impl Sortable {
    /// `sort` overrides the automatic sort value when non-empty.
    pub fn new(value: &str, sort: Option<&str>) -> Sortable {
        let sort = match sort.filter(|s| !s.is_empty()) {
            Some(s) => sort_field(s),
            None => artist_sort(value), // useOfficialSortValueIfFound
        };
        Sortable { value: value.to_string(), sort }
    }
}

/// File-name safe text: no diacritics or punctuation, whitespace as dashes.
pub fn file_name(input: &str) -> String {
    let mut out = String::new();
    let mut last_space = false;
    for c in input.chars() {
        let c = strip_diacritic(c);
        if c.is_whitespace() {
            if !last_space {
                out.push('-');
            }
            last_space = true;
        } else if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
            last_space = false;
        }
    }
    out
}

fn strip_diacritic(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'A',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'È' | 'É' | 'Ê' | 'Ë' => 'E',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'Ì' | 'Í' | 'Î' | 'Ï' => 'I',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' => 'O',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'Ù' | 'Ú' | 'Û' | 'Ü' => 'U',
        'ý' | 'ÿ' => 'y',
        'Ý' => 'Y',
        'ñ' => 'n',
        'Ñ' => 'N',
        'ç' => 'c',
        'Ç' => 'C',
        c => c,
    }
}

/// Case-insensitive ASCII prefix test (F#'s `String.startsWith` uses OrdinalIgnoreCase).
pub(crate) fn starts_with_ci(s: &str, prefix: &str) -> bool {
    s.get(..prefix.len()).is_some_and(|x| x.eq_ignore_ascii_case(prefix))
}

/// Case-insensitive ASCII suffix test (F#'s `String.endsWith` uses OrdinalIgnoreCase).
pub(crate) fn ends_with_ci(s: &str, suffix: &str) -> bool {
    s.len() >= suffix.len() && s.get(s.len() - suffix.len()..).is_some_and(|x| x.eq_ignore_ascii_case(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_values() {
        assert_eq!(artist_sort("The Beatles"), "Beatles");
        assert_eq!(artist_sort("Mötley Crüe"), "Motley Crue");
        assert_eq!(title_sort("...And Justice"), "And Justice");
        assert_eq!(dlc_key("Fluid Love!"), "FluidLove");
        assert_eq!(file_name("Mötley  Crüe: Live"), "Motley-Crue-Live");
    }
}
