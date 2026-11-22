#include <stdint.h>
#include <stddef.h>

/* OmniKVM ESP32-S3 USB HID Dongle Firmware Skeleton (esparrier compatible) */
typedef struct __attribute__((packed)) {
    uint8_t buttons;
    int8_t dx;
    int8_t dy;
    int8_t wheel;
} omnikvm_usb_mouse_report_t;

typedef struct __attribute__((packed)) {
    uint8_t modifiers;
    uint8_t reserved;
    uint8_t keycodes[6];
} omnikvm_usb_kbd_report_t;

void omnikvm_usb_hid_send_mouse(const omnikvm_usb_mouse_report_t *report) {
    (void)report;
    /* TinyUSB tud_hid_mouse_report(HID_ITF_PROTOCOL_MOUSE, ...) */
}

void omnikvm_usb_hid_send_keyboard(const omnikvm_usb_kbd_report_t *report) {
    (void)report;
    /* TinyUSB tud_hid_keyboard_report(HID_ITF_PROTOCOL_KEYBOARD, ...) */
}
