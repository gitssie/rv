/* Deterministic host PhotoKit stand-in for the TrollVNC protocol fixture. */
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static unsigned char fake_op;
static char fake_root[4096];
static char fake_value[4096];
static char fake_imported_name[256] = "fixture.png";
static int fake_expected_hash;
static int fake_deleted;
static unsigned fake_polls;

int tvPhotoStart(unsigned char op, const char *root, const char *value,
                 const char *expected_sha256, char *token, size_t token_capacity,
                 char *error, size_t error_capacity) {
    if ((op < 4 || (op > 6 && op != 9)) || !root || !value || strlen(root) > 3000 ||
        strlen(value) > 3000 || token_capacity < 37) {
        snprintf(error, error_capacity, "invalid fake photo request");
        return -1;
    }
    fake_op = op;
    fake_expected_hash = expected_sha256 && expected_sha256[0];
    snprintf(fake_root, sizeof(fake_root), "%s", root);
    snprintf(fake_value, sizeof(fake_value), "%s", value);
    fake_polls = 0;
    snprintf(token, token_capacity, "00000000-0000-0000-0000-000000000001");
    return 0;
}

int tvPhotoPoll(const char *token, char **payload, char **error) {
    if (!token || strcmp(token, "00000000-0000-0000-0000-000000000001")) {
        *error = strdup("unknown job");
        return -1;
    }
    if (fake_polls++ == 0)
        return 1;
    if (fake_op == 4) {
        char path[8192];
        snprintf(path, sizeof(path), "%s%s", fake_root, fake_value);
        struct stat info;
        if (stat(path, &info) < 0 || !S_ISREG(info.st_mode)) {
            *error = strdup("image not found");
            return -1;
        }
        if (fake_expected_hash) {
            const char *original = strstr(fake_value, "--");
            if (!original || !original[2] || strlen(original + 2) >= sizeof(fake_imported_name)) {
                *error = strdup("original image name missing from upload");
                return -1;
            }
            snprintf(fake_imported_name, sizeof(fake_imported_name), "%s", original + 2);
        }
        if (strstr(fake_value, "/Media/DCIM/.MISC/Incoming/rv-upload-") == fake_value)
            unlink(path);
        *payload = strdup("{\"assetId\":\"fixture-photo\"}");
    } else if (fake_op == 5) {
        if (strcmp(fake_value, "0") && !strstr(fake_value, "\"offset\":0")) {
            *error = strdup("invalid album page request");
            return -1;
        }
        const char *name = strstr(fake_value, "\"album\":\"fixture-album\"") ? "album-photo.png" : fake_imported_name;
        char result[1024];
        if (fake_deleted)
            snprintf(result, sizeof(result), "{\"total\":0,\"offset\":0,\"albums\":[{\"id\":\"fixture-album\",\"name\":\"Fixture album\"}],\"entries\":[]}");
        else
            snprintf(result, sizeof(result), "{\"total\":2,\"offset\":0,\"albums\":[{\"id\":\"fixture-album\",\"name\":\"Fixture album\"}],\"entries\":[{\"id\":\"fixture-photo\",\"name\":\"%s\",\"date\":1720000000,\"width\":64,\"height\":64,\"thumbnail\":\"\"},{\"id\":\"fixture-photo-2\",\"name\":\"second.png\",\"date\":1710000000,\"width\":32,\"height\":32,\"thumbnail\":\"\"}]}", name);
        *payload = strdup(result);
    } else if (fake_op == 9) {
        if (strcmp(fake_value, "fixture-cancel") == 0) {
            *error = strdup("Photo deletion cancelled");
            return -2;
        }
        if ((!strstr(fake_value, "fixture-photo") ||
             (fake_value[0] == '{' && !strstr(fake_value, "fixture-photo-2"))) || fake_deleted) {
            *error = strdup("photo no longer exists");
            return -1;
        }
        fake_deleted = 1;
        *payload = fake_value[0] == '{'
            ? strdup("{\"deleted\":[\"fixture-photo\",\"fixture-photo-2\"]}")
            : strdup("{\"deleted\":\"fixture-photo\"}");
    } else {
        char folder[8192];
        snprintf(folder, sizeof(folder), "%s/Media", fake_root);
        mkdir(folder, 0700);
        snprintf(folder, sizeof(folder), "%s/Media/DCIM", fake_root);
        mkdir(folder, 0700);
        snprintf(folder, sizeof(folder), "%s/Media/DCIM/.MISC", fake_root);
        mkdir(folder, 0700);
        snprintf(folder, sizeof(folder), "%s/Media/DCIM/.MISC/Incoming", fake_root);
        mkdir(folder, 0700);
        char path[8192];
        snprintf(path, sizeof(path), "%s/rv-export-fixture.png", folder);
        FILE *file = fopen(path, "wb");
        if (!file || fwrite("photo", 1, 5, file) != 5) {
            if (file) fclose(file);
            *error = strdup("cannot stage fake export");
            return -1;
        }
        fclose(file);
        *payload = strdup("{\"path\":\"/Media/DCIM/.MISC/Incoming/rv-export-fixture.png\",\"name\":\"fixture.png\",\"size\":5}");
    }
    return 0;
}

int tvPhotoCleanupExport(const char *remote_path) {
    if (!remote_path || strcmp(remote_path, "/Media/DCIM/.MISC/Incoming/rv-export-fixture.png")) {
        errno = EINVAL;
        return -1;
    }
    char path[8192];
    snprintf(path, sizeof(path), "%s%s", fake_root, remote_path);
    return unlink(path);
}
