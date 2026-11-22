fn main() {
    println!("cargo:rerun-if-changed=../../firmware/esp32-s3/usb_hid.c");
    println!("cargo:rerun-if-changed=../../firmware/esp32-s3/ble_gap.c");

    cc::Build::new()
        .file("../../firmware/esp32-s3/usb_hid.c")
        .file("../../firmware/esp32-s3/ble_gap.c")
        .warnings(true)
        .extra_warnings(true)
        .warnings_into_errors(true)
        .compile("omnikvm_esp32_fw");
}
