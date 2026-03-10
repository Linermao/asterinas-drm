// SPDX-License-Identifier: MIT
/*
 * Test program for DRM page flip event delivery
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <stdint.h>
#include <errno.h>
#include <poll.h>
#include <sys/ioctl.h>
#include <drm/drm.h>
#include <drm/drm_fourcc.h>

#define DRM_DEV "/dev/dri/card0"

struct drm_test_context {
    int fd;
    uint32_t crtc_id;
    uint32_t connector_id;
    uint32_t encoder_id;
    uint32_t fb_id[2];
    struct drm_mode_modeinfo mode;
};

static int drm_open(const char *device)
{
    int fd = open(device, O_RDWR | O_CLOEXEC);
    if (fd < 0) {
        perror("Failed to open DRM device");
        return -1;
    }
    printf("[+] Opened DRM device: %s (fd=%d)\n", device, fd);
    return fd;
}

static int drm_get_resources(int fd, struct drm_test_context *ctx)
{
    struct drm_mode_card_res res = {0};
    int ret;

    // First call: get counts
    ret = ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &res);
    if (ret < 0) {
        perror("DRM_IOCTL_MODE_GETRESOURCES (count)");
        return -1;
    }

    printf("[+] Resources: %d crtcs, %d encoders, %d connectors, %d fbs\n",
           res.count_crtcs, res.count_encoders, res.count_connectors, res.count_fbs);

    if (res.count_crtcs == 0 || res.count_connectors == 0) {
        fprintf(stderr, "No CRTCs or connectors available\n");
        return -1;
    }

    // Allocate arrays
    uint32_t *crtcs = calloc(res.count_crtcs, sizeof(uint32_t));
    uint32_t *encoders = calloc(res.count_encoders, sizeof(uint32_t));
    uint32_t *connectors = calloc(res.count_connectors, sizeof(uint32_t));
    uint32_t *fbs = calloc(res.count_fbs, sizeof(uint32_t));

    if (!crtcs || !encoders || !connectors || !fbs) {
        perror("calloc");
        free(crtcs);
        free(encoders);
        free(connectors);
        free(fbs);
        return -1;
    }

    // Second call: get IDs
    res.crtc_id_ptr = (unsigned long)crtcs;
    res.encoder_id_ptr = (unsigned long)encoders;
    res.connector_id_ptr = (unsigned long)connectors;
    res.fb_id_ptr = (unsigned long)fbs;

    ret = ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &res);
    if (ret < 0) {
        perror("DRM_IOCTL_MODE_GETRESOURCES (get)");
        free(crtcs);
        free(encoders);
        free(connectors);
        free(fbs);
        return -1;
    }

    // Use first available crtc and connector
    ctx->crtc_id = crtcs[0];
    ctx->connector_id = connectors[0];
    ctx->encoder_id = encoders[0];

    printf("[+] Using CRTC %u, Connector %u, Encoder %u\n",
           ctx->crtc_id, ctx->connector_id, ctx->encoder_id);

    free(crtcs);
    free(encoders);
    free(connectors);
    free(fbs);

    return 0;
}

static int drm_get_connector_mode(int fd, uint32_t connector_id,
                                   struct drm_mode_modeinfo *mode)
{
    struct drm_mode_get_connector conn = {0};
    int ret;

    conn.connector_id = connector_id;

    // First call: get counts
    ret = ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &conn);
    if (ret < 0) {
        perror("DRM_IOCTL_MODE_GETCONNECTOR (count)");
        return -1;
    }

    if (conn.count_modes == 0) {
        fprintf(stderr, "No modes available on connector\n");
        return -1;
    }

    printf("[+] Connector has %d modes, %d encoders, %d props\n",
           conn.count_modes, conn.count_encoders, conn.count_props);

    // Allocate ALL arrays (modes, encoders, props)
    struct drm_mode_modeinfo *modes = calloc(conn.count_modes, sizeof(*modes));
    uint32_t *encoders = calloc(conn.count_encoders, sizeof(uint32_t));
    uint32_t *props = calloc(conn.count_props, sizeof(uint32_t));
    uint64_t *prop_values = calloc(conn.count_props, sizeof(uint64_t));

    if (!modes || !encoders || !props || !prop_values) {
        perror("calloc connector data");
        free(modes);
        free(encoders);
        free(props);
        free(prop_values);
        return -1;
    }

    // Second call: get all data
    conn.modes_ptr = (unsigned long)modes;
    conn.encoders_ptr = (unsigned long)encoders;
    conn.props_ptr = (unsigned long)props;
    conn.prop_values_ptr = (unsigned long)prop_values;

    ret = ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &conn);
    if (ret < 0) {
        perror("DRM_IOCTL_MODE_GETCONNECTOR (get)");
        free(modes);
        free(encoders);
        free(props);
        free(prop_values);
        return -1;
    }

    // Use first mode (usually preferred)
    *mode = modes[0];
    printf("[+] Using mode: %ux%u@%uHz\n",
           mode->hdisplay, mode->vdisplay, mode->vrefresh);

    free(modes);
    free(encoders);
    free(props);
    free(prop_values);

    return 0;
}

static int drm_create_dumb_buffer(int fd, uint32_t width, uint32_t height,
                                   uint32_t *handle, uint32_t *pitch, uint64_t *size)
{
    struct drm_mode_create_dumb create = {0};

    create.width = width;
    create.height = height;
    create.bpp = 32;

    int ret = ioctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &create);
    if (ret < 0) {
        perror("DRM_IOCTL_MODE_CREATE_DUMB");
        return -1;
    }

    *handle = create.handle;
    *pitch = create.pitch;
    *size = create.size;

    printf("[+] Created dumb buffer: handle=%u, pitch=%u, size=%lu\n",
           *handle, *pitch, *size);

    return 0;
}

static int drm_add_fb(int fd, uint32_t handle, uint32_t width, uint32_t height,
                       uint32_t pitch, uint32_t *fb_id)
{
    struct drm_mode_fb_cmd fb = {0};

    fb.width = width;
    fb.height = height;
    fb.pitch = pitch;
    fb.bpp = 32;
    fb.depth = 24;
    fb.handle = handle;

    int ret = ioctl(fd, DRM_IOCTL_MODE_ADDFB, &fb);
    if (ret < 0) {
        perror("DRM_IOCTL_MODE_ADDFB");
        return -1;
    }

    *fb_id = fb.fb_id;
    printf("[+] Added framebuffer: fb_id=%u\n", *fb_id);

    return 0;
}

static int drm_set_crtc(int fd, uint32_t fb_id, uint32_t crtc_id,
                         uint32_t connector_id, uint32_t encoder_id,
                         struct drm_mode_modeinfo *mode)
{
    struct drm_mode_crtc crtc = {0};

    crtc.crtc_id = crtc_id;
    crtc.fb_id = fb_id;
    crtc.set_connectors_ptr = (unsigned long)&connector_id;
    crtc.count_connectors = 1;
    crtc.mode_valid = 1;
    memcpy(&crtc.mode, mode, sizeof(*mode));

    int ret = ioctl(fd, DRM_IOCTL_MODE_SETCRTC, &crtc);
    if (ret < 0) {
        perror("DRM_IOCTL_MODE_SETCRTC");
        return -1;
    }

    printf("[+] Set CRTC %u to framebuffer %u\n", crtc_id, fb_id);
    return 0;
}

static int drm_page_flip(int fd, uint32_t fb_id, uint32_t crtc_id,
                          void *user_data)
{
    struct drm_mode_crtc_page_flip flip = {0};

    flip.crtc_id = crtc_id;
    flip.fb_id = fb_id;
    flip.flags = DRM_MODE_PAGE_FLIP_EVENT;
    flip.user_data = (unsigned long)user_data;

    printf("[*] Issuing page flip to fb %u (user_data=%p)...\n", fb_id, user_data);

    int ret = ioctl(fd, DRM_IOCTL_MODE_PAGE_FLIP, &flip);
    if (ret < 0) {
        perror("DRM_IOCTL_MODE_PAGE_FLIP");
        return -1;
    }

    printf("[+] Page flip issued successfully\n");
    return 0;
}

static int drm_wait_for_event(int fd, unsigned int timeout_ms)
{
    struct pollfd pfd = {
        .fd = fd,
        .events = POLLIN,
    };

    printf("[*] Waiting for event (timeout=%ums)...\n", timeout_ms);

    int ret = poll(&pfd, 1, timeout_ms);
    if (ret < 0) {
        perror("poll");
        return -1;
    }

    if (ret == 0) {
        fprintf(stderr, "Timeout: no event received\n");
        return -1;
    }

    if (pfd.revents & POLLIN) {
        printf("[+] Event available (POLLIN)\n");
        return 0;
    }

    fprintf(stderr, "Unexpected poll event: 0x%x\n", pfd.revents);
    return -1;
}

static int drm_read_event(int fd, void *expected_user_data)
{
    struct drm_event_vblank ev = {0};
    ssize_t n;

    printf("[*] Reading event...\n");

    n = read(fd, &ev, sizeof(ev));
    if (n < 0) {
        perror("read event");
        return -1;
    }

    if (n != sizeof(ev)) {
        fprintf(stderr, "Partial read: %zd bytes (expected %zu)\n", n, sizeof(ev));
        return -1;
    }

    printf("[+] Read %zd bytes\n", n);
    printf("[+] Event details:\n");
    printf("    type = %u\n", ev.base.type);
    printf("    length = %u\n", ev.base.length);
    printf("    user_data = 0x%llx\n", (unsigned long long)ev.user_data);
    printf("    sequence = %u\n", ev.sequence);
    printf("    tv_sec = %u\n", ev.tv_sec);
    printf("    tv_usec = %u\n", ev.tv_usec);
    printf("    crtc_id = %u\n", ev.crtc_id);

    // Validate event type
    if (ev.base.type != DRM_EVENT_FLIP_COMPLETE &&
        ev.base.type != DRM_EVENT_VBLANK) {
        fprintf(stderr, "Unexpected event type: %u\n", ev.base.type);
        return -1;
    }

    // Validate user_data
    if (ev.user_data != (unsigned long)expected_user_data) {
        fprintf(stderr, "user_data mismatch: expected %p, got 0x%llx\n",
                expected_user_data, (unsigned long long)ev.user_data);
        return -1;
    }

    printf("[+] Event validated successfully!\n");
    return 0;
}

static void drm_destroy_dumb_buffer(int fd, uint32_t handle)
{
    struct drm_mode_destroy_dumb destroy = { .handle = handle };
    ioctl(fd, DRM_IOCTL_MODE_DESTROY_DUMB, &destroy);
}

static int drm_rm_fb(int fd, uint32_t fb_id)
{
    struct drm_mode_fb_cmd fb = { .fb_id = fb_id };
    return ioctl(fd, DRM_IOCTL_MODE_RMFB, &fb);
}

int main(int argc, char *argv[])
{
    struct drm_test_context ctx = {0};
    const char *device = DRM_DEV;
    uint32_t width, height;
    uint32_t handles[2] = {0};
    uint64_t sizes[2] = {0};
    uint32_t pitches[2] = {0};
    int ret = 0;
    const void *user_data = (void *)0xDEADBEEF;

    if (argc > 1) {
        device = argv[1];
    }

    printf("=== DRM Page Flip Event Test ===\n");
    printf("Device: %s\n\n", device);

    // Open device
    ctx.fd = drm_open(device);
    if (ctx.fd < 0) {
        return 1;
    }

    // Get resources
    if (drm_get_resources(ctx.fd, &ctx) < 0) {
        ret = 1;
        goto cleanup;
    }

    // Get connector mode
    if (drm_get_connector_mode(ctx.fd, ctx.connector_id, &ctx.mode) < 0) {
        ret = 1;
        goto cleanup;
    }

    width = ctx.mode.hdisplay;
    height = ctx.mode.vdisplay;

    // Create two dumb buffers for double buffering
    printf("\n=== Creating dumb buffers ===\n");
    for (int i = 0; i < 2; i++) {
        if (drm_create_dumb_buffer(ctx.fd, width, height, &handles[i], &pitches[i], &sizes[i]) < 0) {
            ret = 1;
            goto cleanup;
        }

        if (drm_add_fb(ctx.fd, handles[i], width, height, pitches[i], &ctx.fb_id[i]) < 0) {
            ret = 1;
            goto cleanup;
        }
    }

    // Set initial mode
    printf("\n=== Setting initial CRTC ===\n");
    if (drm_set_crtc(ctx.fd, ctx.fb_id[0], ctx.crtc_id,
                      ctx.connector_id, ctx.encoder_id, &ctx.mode) < 0) {
        ret = 1;
        goto cleanup;
    }

    // Give display time to settle
    printf("\n[*] Waiting 1 second for display to settle...\n");
    sleep(1);

    // Perform page flip with event
    printf("\n=== Testing page flip with event ===\n");
    if (drm_page_flip(ctx.fd, ctx.fb_id[1], ctx.crtc_id, (void *)user_data) < 0) {
        ret = 1;
        goto cleanup;
    }

    // Wait for event (Phase 1: should be immediate)
    if (drm_wait_for_event(ctx.fd, 1000) < 0) {
        fprintf(stderr, "Failed to wait for event\n");
        ret = 1;
        goto cleanup;
    }

    // Read and validate event
    if (drm_read_event(ctx.fd, user_data) < 0) {
        fprintf(stderr, "Failed to read/validate event\n");
        ret = 1;
        goto cleanup;
    }

    printf("\n=== TEST PASSED ===\n");
    printf("Page flip event delivery is working!\n");

cleanup:
    // Cleanup
    printf("\n=== Cleanup ===\n");
    for (int i = 0; i < 2; i++) {
        if (ctx.fb_id[i] != 0) {
            drm_rm_fb(ctx.fd, ctx.fb_id[i]);
        }
        if (handles[i] != 0) {
            drm_destroy_dumb_buffer(ctx.fd, handles[i]);
        }
    }

    if (ctx.fd >= 0) {
        close(ctx.fd);
    }

    return ret;
}
