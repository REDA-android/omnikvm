#include <stdint.h>
#include <stddef.h>

/* OmniKVM ESP32-S3 USB HID Dongle Firmware (esparrier compatible) */
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
    /* TinyUSB: tud_hid_mouse_report(HID_ITF_PROTOCOL_MOUSE, report->buttons, report->dx, report->dy, report->wheel, 0); */
}

void omnikvm_usb_hid_send_keyboard(const omnikvm_usb_kbd_report_t *report) {
    (void)report;
    /* TinyUSB: tud_hid_keyboard_report(HID_ITF_PROTOCOL_KEYBOARD, report->modifiers, report->keycodes); */
}

int omnikvm_esp_dispatch_frame(const uint8_t *buf, size_t len) {
    if (!buf || len < 8) {
        return -1;
    }
    if (buf[0] != 0xAA || buf[1] != 0x55) {
        return -2;
    }
    uint8_t cmd = buf[2];
    uint8_t plen = buf[3];
    if (len < (size_t)(4 + plen + 4)) {
        return -3;
    }
    if (cmd == 0x01 && plen == sizeof(omnikvm_usb_mouse_report_t)) {
        const omnikvm_usb_mouse_report_t *m = (const omnikvm_usb_mouse_report_t *)(buf + 4);
        omnikvm_usb_hid_send_mouse(m);
        return 0;
    }
    if (cmd == 0x02 && plen == sizeof(omnikvm_usb_kbd_report_t)) {
        const omnikvm_usb_kbd_report_t *k = (const omnikvm_usb_kbd_report_t *)(buf + 4);
        omnikvm_usb_hid_send_keyboard(k);
        return 0;
    }
    return 1;
}
