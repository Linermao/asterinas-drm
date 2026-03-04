#include <string.h>
#include <stdio.h>
#include <fcntl.h>
#include <stdint.h>
#include <errno.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>
#include <sys/ioctl.h>
#include <sys/mman.h>

#include <xf86drm.h>
#include <xf86drmMode.h>
#include <drm/drm.h>
#include <drm/drm_mode.h>

int main(int argc, char **argv) {
    const char *card;
    int ret;
	struct modeset_dev *iter;
    struct modeset_dev *dev;
	struct modeset_buf *buf;

      /* check which DRM device to open */
    if (argc > 1)
        card = argv[1];
    else
        card = "/dev/dri/card0";

    int fd = open(card, O_RDWR);
    if (fd < 0) {
        perror("open");
        return 1;
    }

    drmModeRes *res;
    res = drmModeGetResources(fd);
    if (!res) {
        fprintf(stderr, "cannot retrieve DRM resources (%d): %m\n",
            errno);
        return -errno;
    }

    printf("get resources correct!\n");
    printf("counts: connectors=%u, crtcs=%u, fbs=%u, encoders=%u\n",
           res->count_connectors, res->count_crtcs, res->count_fbs, res->count_encoders);

    drmModeFreeResources(res);
    close(fd);
    return 0;
}
