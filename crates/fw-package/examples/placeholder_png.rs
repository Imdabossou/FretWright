//! Writes the placeholder album art for an artist and title as a PNG.
#![allow(clippy::unwrap_used)]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    fw_package::dds::placeholder(&a[1], &a[2]).save(&a[3]).unwrap();
}
