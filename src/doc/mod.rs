const DOC_JSON_ZZ: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/doc.json.zz"));

#[must_use]
pub fn json() -> Vec<u8> {
    miniz_oxide::inflate::decompress_to_vec_zlib(DOC_JSON_ZZ)
        .expect("failed to decompress doc.json")
}
