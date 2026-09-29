/* Host-side TightVNC file-transfer fixture using TrollVNC's LibVNCServer. */
#include <rfb/rfb.h>
#include <arpa/inet.h>
#include <stdio.h>
#include <stdlib.h>

#include "../../../TrollVNC/src/ClipboardText.h"

/* The TightVNC extension exports these root helpers but omits public headers. */
extern void InitFileTransfer(void);
extern int SetFtpRoot(char *path);
extern void tvRegisterFileManagement(const char *root);

static char clipboard_path[4096];
static char utf8_wire_path[4096];
static char latin1_clipboard_path[4096];

static void write_clipboard(const char *path, char *text, int length) {
    FILE *output = fopen(path, "wb");
    if (output) {
        fwrite(text, 1, (size_t)length, output);
        fclose(output);
    }
}

static void capture_utf8_clipboard(char *text, int length, rfbClientPtr client) {
    (void)client;
    write_clipboard(utf8_wire_path, text, length);
    write_clipboard(clipboard_path, text, tvClipboardUTF8TextLength(text, length));
}

static void capture_latin1_clipboard(char *text, int length, rfbClientPtr client) {
    (void)client;
    write_clipboard(latin1_clipboard_path, text, length);
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: %s DIRECTORY PORT\n", argv[0]);
        return 2;
    }
    int port = atoi(argv[2]);
    if (port < 1 || port > 65535) {
        fprintf(stderr, "invalid port\n");
        return 2;
    }
    int rfb_argc = 1;
    char *rfb_argv[] = {argv[0], NULL};
    rfbScreenInfoPtr screen = rfbGetScreen(&rfb_argc, rfb_argv, 8, 8, 8, 3, 4);
    if (screen == NULL) {
        fprintf(stderr, "rfbGetScreen failed\n");
        return 1;
    }
    screen->frameBuffer = calloc(8 * 8, 4);
    screen->port = port;
    screen->ipv6port = 0;
    screen->listenInterface = htonl(INADDR_LOOPBACK);
    screen->alwaysShared = TRUE;
    screen->permitFileTransfer = TRUE;
    snprintf(clipboard_path, sizeof(clipboard_path), "%s/clipboard.txt", argv[1]);
    snprintf(utf8_wire_path, sizeof(utf8_wire_path), "%s/clipboard-wire.bin", argv[1]);
    snprintf(latin1_clipboard_path, sizeof(latin1_clipboard_path), "%s/clipboard-latin1.txt", argv[1]);
    screen->setXCutTextUTF8 = capture_utf8_clipboard;
    screen->setXCutText = capture_latin1_clipboard;
    rfbRegisterTightVNCFileTransferExtension();
    tvRegisterFileManagement(argv[1]);
    InitFileTransfer();
    if (!SetFtpRoot(argv[1])) {
        fprintf(stderr, "cannot set file transfer root\n");
        return 2;
    }
    rfbInitServer(screen);
    while (rfbIsActive(screen)) {
        rfbProcessEvents(screen, 10000);
    }
    rfbScreenCleanup(screen);
    return 0;
}
