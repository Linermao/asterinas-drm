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

#include "../util/common.h"
#include "../util/format.h"
#include "../util/kms.h"
#include "../util/pattern.h"

int main(int argc, char **argv) {
	struct device dev;
	int encoders = 1, connectors = 1, crtcs = 1, planes = 1, framebuffers = 1;

    const char *card;
    int ret;

    /* check which DRM device to open */
    if (argc > 1)
        card = argv[1];
    else
        card = "/dev/dri/card0";

    dev.fd = open(card, O_RDWR);
    if (dev.fd < 0) {
        perror("open");
        return 1;
    }

    dump_getversion(dev.fd);
    // dump_getcap(dev.fd);

	dev.resources = get_resources(&dev);
	if (!dev.resources) {
		drmClose(dev.fd);
		return 1;
	}

#define dump_resource(dev, res) if (res) dump_##res(dev)

    dump_resource(&dev, connectors);
    dump_resource(&dev, encoders);
	dump_resource(&dev, crtcs);
	dump_resource(&dev, planes);
	dump_resource(&dev, framebuffers);
}