/// A CoT type string such as `a-f-G-U-C`, read as dash-separated segments.
///
/// Atoms (`a-...`) are things on the map: the second segment is affiliation, the third is battle
/// dimension, and the rest is the MIL-STD-2525 function hierarchy. Other roots are bits (`b`,
/// messages and markers), tasking (`t`) and user drawings (`u`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CotType<'a>(&'a str);

impl<'a> CotType<'a> {
    pub fn new(s: &'a str) -> Self {
        Self(s)
    }

    pub fn as_str(&self) -> &'a str {
        self.0
    }

    pub fn segments(&self) -> impl Iterator<Item = &'a str> {
        self.0.split('-')
    }

    pub fn is_atom(&self) -> bool {
        self.segments().next() == Some("a")
    }

    pub fn affiliation(&self) -> Option<Affiliation> {
        self.atom_segment(1).and_then(Affiliation::from_code)
    }

    pub fn dimension(&self) -> Option<Dimension> {
        self.atom_segment(2).and_then(Dimension::from_code)
    }

    /// Function segments after the dimension, e.g. `["U", "C"]` for `a-f-G-U-C`.
    pub fn function(&self) -> Vec<&'a str> {
        if self.is_atom() { self.segments().skip(3).collect() } else { Vec::new() }
    }

    /// Hierarchical prefix match on whole segments, where `.` in `pattern` matches any one
    /// segment. `a-f-G-U-C` matches `a-f`, `a-.-G` and itself, but not `a-f-G-U-C-I` or `a-fo`.
    pub fn matches(&self, pattern: &str) -> bool {
        let mut mine = self.segments();
        pattern.split('-').all(|p| mine.next().is_some_and(|m| p == "." || p == m))
    }

    /// 15-character MIL-STD-2525C symbol ID for rendering (e.g. with milsymbol). `None` for
    /// non-atoms or an unrecognized affiliation or dimension.
    pub fn to_sidc(&self) -> Option<String> {
        let affiliation = self.affiliation()?;
        let dimension = self.dimension()?;
        let mut function: String = self.function().concat().to_ascii_uppercase();
        // 2525C function IDs are six characters; anything deeper isn't a symbol it defines, so
        // fall back to the bare frame rather than drawing the wrong icon.
        if function.len() > 6 || !function.chars().all(|c| c.is_ascii_alphanumeric()) {
            function.clear();
        }
        let mut sidc = format!("S{}{}P{function:-<6}", affiliation.sidc_code(), dimension.code());
        // Ground installations carry the H modifier in 2525C; without it they render as units.
        sidc.push(if dimension == Dimension::Ground && function.starts_with('I') { 'H' } else { '-' });
        Some(format!("{sidc:-<15}"))
    }

    /// Short human label: the well-known name, else affiliation and dimension ("Hostile air").
    pub fn label(&self) -> Option<String> {
        self.description()
            .map(String::from)
            .or_else(|| Some(format!("{} {}", self.affiliation()?.name(), self.dimension()?.name())))
    }

    /// Human name for well-known non-atom types.
    pub fn description(&self) -> Option<&'static str> {
        WELL_KNOWN.iter().find(|(t, _)| *t == self.0).map(|(_, d)| *d)
    }

    fn atom_segment(&self, index: usize) -> Option<char> {
        let seg = self.segments().nth(index).filter(|_| self.is_atom())?;
        let mut chars = seg.chars();
        chars.next().filter(|_| chars.next().is_none())
    }
}

const WELL_KNOWN: &[(&str, &str)] = &[
    ("b-t-f", "GeoChat message"),
    ("b-t-f-d", "GeoChat delivered receipt"),
    ("b-t-f-r", "GeoChat read receipt"),
    ("b-m-p-w", "Waypoint"),
    ("b-m-r", "Route"),
    ("b-m-p-s-p-i", "Spot marker"),
    ("b-a-o-tbl", "Emergency: 911"),
    ("b-a-o-pan", "Emergency: ring the bell"),
    ("b-a-o-opn", "Emergency: troops in contact"),
    ("b-a-o-can", "Emergency cancelled"),
    ("b-f-t-r", "File transfer request"),
    ("t-x-c-t", "Ping"),
    ("t-x-c-t-r", "Ping reply"),
    ("t-x-d-d", "Delete"),
    ("t-x-takp-v", "TAK protocol support"),
    ("t-x-takp-q", "TAK protocol request"),
    ("t-x-takp-r", "TAK protocol response"),
    ("u-d-f", "Freehand drawing"),
    ("u-d-r", "Rectangle"),
    ("u-d-c-c", "Circle"),
    ("u-rb-a", "Range and bearing line"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Affiliation {
    Pending,
    Unknown,
    AssumedFriend,
    Friend,
    Neutral,
    Suspect,
    Hostile,
    Joker,
    Faker,
    NoneSpecified,
    Other,
}

impl Affiliation {
    pub fn from_code(c: char) -> Option<Self> {
        Some(match c {
            'p' => Self::Pending,
            'u' => Self::Unknown,
            'a' => Self::AssumedFriend,
            'f' => Self::Friend,
            'n' => Self::Neutral,
            's' => Self::Suspect,
            'h' => Self::Hostile,
            'j' => Self::Joker,
            'k' => Self::Faker,
            'o' => Self::NoneSpecified,
            'x' => Self::Other,
            _ => return None,
        })
    }

    pub fn code(self) -> char {
        match self {
            Self::Pending => 'p',
            Self::Unknown => 'u',
            Self::AssumedFriend => 'a',
            Self::Friend => 'f',
            Self::Neutral => 'n',
            Self::Suspect => 's',
            Self::Hostile => 'h',
            Self::Joker => 'j',
            Self::Faker => 'k',
            Self::NoneSpecified => 'o',
            Self::Other => 'x',
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Unknown => "Unknown",
            Self::AssumedFriend => "Assumed friend",
            Self::Friend => "Friend",
            Self::Neutral => "Neutral",
            Self::Suspect => "Suspect",
            Self::Hostile => "Hostile",
            Self::Joker => "Joker",
            Self::Faker => "Faker",
            Self::NoneSpecified => "Unspecified",
            Self::Other => "Other",
        }
    }

    fn sidc_code(self) -> char {
        match self {
            // 2525C has no "other" affiliation.
            Self::Other => 'U',
            a => a.code().to_ascii_uppercase(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Space,
    Air,
    Ground,
    SeaSurface,
    Subsurface,
    Sof,
    Other,
}

impl Dimension {
    pub fn from_code(c: char) -> Option<Self> {
        Some(match c {
            'P' => Self::Space,
            'A' => Self::Air,
            'G' => Self::Ground,
            'S' => Self::SeaSurface,
            'U' => Self::Subsurface,
            'F' => Self::Sof,
            'X' => Self::Other,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Space => "space",
            Self::Air => "air",
            Self::Ground => "ground",
            Self::SeaSurface => "sea surface",
            Self::Subsurface => "subsurface",
            Self::Sof => "SOF",
            Self::Other => "other",
        }
    }

    pub fn code(self) -> char {
        match self {
            Self::Space => 'P',
            Self::Air => 'A',
            Self::Ground => 'G',
            Self::SeaSurface => 'S',
            Self::Subsurface => 'U',
            Self::Sof => 'F',
            Self::Other => 'X',
        }
    }
}
