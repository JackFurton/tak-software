use cot::{Affiliation, CotType, Dimension};

#[test]
fn reads_atom_segments() {
    let t = CotType::new("a-h-A-M-F-Q");
    assert!(t.is_atom());
    assert_eq!(t.affiliation(), Some(Affiliation::Hostile));
    assert_eq!(t.dimension(), Some(Dimension::Air));
    assert_eq!(t.function(), ["M", "F", "Q"]);

    let chat = CotType::new("b-t-f");
    assert!(!chat.is_atom());
    assert_eq!(chat.affiliation(), None);
    assert_eq!(chat.description(), Some("GeoChat message"));
}

#[test]
fn matches_whole_segments_with_wildcards() {
    let t = CotType::new("a-f-G-U-C");
    for pattern in ["a", "a-f", "a-.-G", "a-f-G-U-C", ".-.-G"] {
        assert!(t.matches(pattern), "{pattern}");
    }
    for pattern in ["a-h", "a-fo", "a-f-G-U-C-I", "b"] {
        assert!(!t.matches(pattern), "{pattern}");
    }
}

#[test]
fn converts_to_2525c_sidc() {
    let sidc = |s| CotType::new(s).to_sidc();
    assert_eq!(sidc("a-f-G-U-C-I").as_deref(), Some("SFGPUCI--------"));
    assert_eq!(sidc("a-h-A").as_deref(), Some("SHAP-----------"));
    assert_eq!(sidc("a-x-S").as_deref(), Some("SUSP-----------"));
    assert_eq!(sidc("a-n-G-I-B").as_deref(), Some("SNGPIB----H----"));
    assert_eq!(sidc("a-f-G-U-C-I-Z-Z-Z-Z").as_deref(), Some("SFGP-----------"));
    assert_eq!(sidc("b-t-f"), None);
    assert_eq!(sidc("a-q-G"), None);
}
