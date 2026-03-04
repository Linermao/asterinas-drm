// SPDX-License-Identifier: MIT
/*
 * Simple vblank test program
 *
 * Tests:
 * 1. Page flip with event
 * 2. Wait for vblank event
 * 3. Check sequence/timestamp
 * 4. Repeat to verify counter increments
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <poll.h>
#include <sys/ioctl.h>
#include <time.h>
#include <drm/drm.h>
#include <drm/drm_fourcc.h>

#define DRM_DEV "/dev/dri/card0"

static int create_dumb_buffer(int fd, uint32_t width, uint32_t height,
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

    return 0;
}

static int add_fb(int fd, uint32_t handle, uint32_t width, uint32_t height,
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

    return 0;
}

static int page_flip(int fd, uint32_t crtc_id, uint32_t fb_id, uint64_t user_data)
{
    struct drm_mode_crtc_page_flip flip = {0};

    flip.crtc_id = crtc_id;
    flip.fb_id = fb_id;
    flip.flags = DRM_MODE_PAGE_FLIP_EVENT;
    flip.user_data = user_data;

    printf("[*] Issuing page flip: fb_id=%u, user_data=0x%llx\n",
           fb_id, (unsigned long long)user_data);

    int ret = ioctl(fd, DRM_IOCTL_MODE_PAGE_FLIP, &flip);
    if (ret < 0) {
        perror("DRM_IOCTL_MODE_PAGE_FLIP");
        return -1;
    }

    printf("[+] Page flip issued\n");
    return 0;
}

static int wait_for_event(int fd, int timeout_ms)
{
    struct pollfd pfd = { .fd = fd, .events = POLLIN };

    printf("[*] Waiting for event (timeout=%dms)...\n", timeout_ms);

    int ret = poll(&pfd, 1, timeout_ms);
    if (ret < 0) {
        perror("poll");
        return -1;
    }

    if (ret == 0) {
        fprintf(stderr, "[-] Timeout: no event received\n");
        return -1;
    }

    if (pfd.revents & POLLIN) {
        printf("[+] Event available (POLLIN)\n");
        return 0;
    }

    fprintf(stderr, "[-] Unexpected poll event: 0x%x\n", pfd.revents);
    return -1;
}

static int read_event(int fd, uint64_t expected_user_data, int *seq_out)
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
        fprintf(stderr, "[-] Partial read: %zd bytes (expected %zu)\n",
                n, sizeof(ev));
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

    // Validate user_data
    if (ev.user_data != expected_user_data) {
        fprintf(stderr, "[-] user_data mismatch: expected 0x%llx, got 0x%llx\n",
                (unsigned long long)expected_user_data,
                (unsigned long long)ev.user_data);
        return -1;
    }

    if (seq_out) {
        *seq_out = ev.sequence;
    }

    printf("[+] Event validated!\n");
    return 0;
}

static void test_vblank(int fd, uint32_t crtc_id, uint32_t fb_id, int iterations)
{
    int i;
    uint64_t user_data = 0xFEED0000;  // User data for this test
    int last_seq = -1;

    printf("\n=== Testing %d page flips with vblank events ===\n\n", iterations);

    for (i = 0; i < iterations; i++) {
        uint64_t current_user_data = user_data + i;
        int seq;

        printf("\n--- Iteration %d/%d ---\n", i + 1, iterations);

        // Issue page flip
        if (page_flip(fd, crtc_id, fb_id, current_user_data) < 0) {
            fprintf(stderr, "[-] Failed to issue page flip\n");
            return;
        }

        // Wait for event
        if (wait_for_event(fd, 1000) < 0) {
            fprintf(stderr, "[-] Failed to wait for event\n");
            return;
        }

        // Read event
        if (read_event(fd, current_user_data, &seq) < 0) {
            fprintf(stderr, "[-] Failed to read event\n");
            return;
        }

        // Check sequence increment
        if (last_seq >= 0 && seq != last_seq + 1) {
            printf("[!] Sequence jumped from %d to %d (expected %d)\n",
                   last_seq, seq, last_seq + 1);
        }

        last_seq = seq;

        // Small delay between flips
        if (i < iterations - 1) {
            printf("[*] Waiting 1 second before next flip...\n");
            sleep(1);
        }
    }

    printf("\n=== Test PASSED ===\n");
    printf("All %d page flips completed successfully!\n", iterations);
    printf("Final sequence number: %d\n", last_seq);
}

int main(int argc, char *argv[])
{
    const char *device = DRM_DEV;
    int fd;
    uint32_t handle, fb_id;
    uint32_t pitch, width = 1024, height = 768;
    uint64_t size;
    int iterations = 3;

    if (argc > 1) {
        device = argv[1];
    }

    if (argc > 2) {
        iterations = atoi(argv[2]);
        if (iterations < 1) iterations = 1;
        if (iterations > 10) iterations = 10;
    }

    printf("=== DRM Vblank Test ===\n");
    printf("Device: %s\n", device);
    printf("Iterations: %d\n\n", iterations);

    // Open device
    fd = open(device, O_RDWR | O_CLOEXEC);
    if (fd < 0) {
        perror("Failed to open DRM device");
        return 1;
    }

    printf("[+] Opened %s (fd=%d)\n", device, fd);

    // Create dumb buffer
    printf("\n=== Creating dumb buffer ===\n");
    if (create_dumb_buffer(fd, width, height, &handle, &pitch, &size) < 0) {
        close(fd);
        return 1;
    }

    // Create framebuffer
    printf("\n=== Creating framebuffer ===\n");
    if (add_fb(fd, handle, width, height, pitch, &fb_id) < 0) {
        close(fd);
        return 1;
    }

    // Get CRTC ID from resources (simplified - use first CRTC)
    uint32_t crtc_id = 1;  // TODO: Query from DRM

    printf("\n[+] Using CRTC %u\n", crtc_id);

    // Run test
    test_vblank(fd, crtc_id, fb_id, iterations);

    // Cleanup
    printf("\n=== Cleanup ===\n");
    close(fd);

    return 0;
}
