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

struct modeset_buf {
	uint32_t width;
	uint32_t height;
	uint32_t stride;
	uint32_t size;
	uint32_t handle;
	uint8_t *map;
	uint32_t fb;
};

struct modeset_dev {
    struct modeset_dev *next;

    unsigned int front_buf;
    struct modeset_buf bufs[2];

    drmModeModeInfo mode;
    uint32_t conn;
    uint32_t crtc;
    drmModeCrtc *saved_crtc;
};

static struct modeset_dev *modeset_list = NULL;

static int modeset_find_crtc(int fd, drmModeRes *res, drmModeConnector *conn,
                 struct modeset_dev *dev)
{
    drmModeEncoder *enc;
    unsigned int i, j;
    int32_t crtc;
    struct modeset_dev *iter;

    /* first try the currently conected encoder+crtc */
    if (conn->encoder_id)
        enc = drmModeGetEncoder(fd, conn->encoder_id);
    else
        enc = NULL;

    if (enc) {
        if (enc->crtc_id) {
            crtc = enc->crtc_id;
            for (iter = modeset_list; iter; iter = iter->next) {
                if (iter->crtc == crtc) {
                    crtc = -1;
                    break;
                }
            }

            if (crtc >= 0) {
                drmModeFreeEncoder(enc);
                dev->crtc = crtc;
                return 0;
            }
        }

        drmModeFreeEncoder(enc);
    }

    /* If the connector is not currently bound to an encoder or if the
     * encoder+crtc is already used by another connector (actually unlikely
     * but lets be safe), iterate all other available encoders to find a
     * matching CRTC. */
    for (i = 0; i < conn->count_encoders; ++i) {
        enc = drmModeGetEncoder(fd, conn->encoders[i]);
        if (!enc) {
            fprintf(stderr, "cannot retrieve encoder %u:%u (%d): %m\n",
                i, conn->encoders[i], errno);
            continue;
        }

        /* iterate all global CRTCs */
        for (j = 0; j < res->count_crtcs; ++j) {
            /* check whether this CRTC works with the encoder */
            if (!(enc->possible_crtcs & (1 << j)))
                continue;

            /* check that no other device already uses this CRTC */
            crtc = res->crtcs[j];
            for (iter = modeset_list; iter; iter = iter->next) {
                if (iter->crtc == crtc) {
                    crtc = -1;
                    break;
                }
            }

            /* we have found a CRTC, so save it and return */
            if (crtc >= 0) {
                drmModeFreeEncoder(enc);
                dev->crtc = crtc;
                return 0;
            }
        }

        drmModeFreeEncoder(enc);
    }

    fprintf(stderr, "cannot find suitable CRTC for connector %u\n",
        conn->connector_id);
    return -ENOENT;
}

static int modeset_create_fb(int fd, struct modeset_buf *buf)
{
    struct drm_mode_create_dumb creq;
    struct drm_mode_destroy_dumb dreq;
    struct drm_mode_map_dumb mreq;
    int ret;

    /* create dumb buffer */
    memset(&creq, 0, sizeof(creq));
    creq.width = buf->width;
    creq.height = buf->height;
    creq.bpp = 32;
    ret = drmIoctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &creq);
    if (ret < 0) {
        fprintf(stderr, "cannot create dumb buffer (%d): %m\n",
            errno);
        return -errno;
    }
    buf->stride = creq.pitch;
    buf->size = creq.size;
    buf->handle = creq.handle;

    /* create framebuffer object for the dumb-buffer */
    ret = drmModeAddFB(fd, buf->width, buf->height, 24, 32, buf->stride,
               buf->handle, &buf->fb);
    if (ret) {
        fprintf(stderr, "cannot create framebuffer (%d): %m\n",
            errno);
        ret = -errno;
        goto err_destroy;
    }

    printf("handle: %d; fb_id: %d\n", buf->handle, buf->fb);

    /* prepare buffer for memory mapping */
    memset(&mreq, 0, sizeof(mreq));
    mreq.handle = buf->handle;
    ret = drmIoctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &mreq);
    if (ret) {
        fprintf(stderr, "cannot map dumb buffer (%d): %m\n",
            errno);
        ret = -errno;
        goto err_fb;
    }

    printf("size: %d; offset: %llu\n", buf->size, mreq.offset);

    /* perform actual memory mapping */
    buf->map = mmap(0, buf->size, PROT_READ | PROT_WRITE, MAP_SHARED,
                fd, mreq.offset);
    if (buf->map == MAP_FAILED) {
        fprintf(stderr, "cannot mmap dumb buffer (%d): %m\n",
            errno);
        ret = -errno;
        goto err_fb;
    }
    printf("mmap returned address: %p\n", buf->map);

    /* clear the framebuffer to 0 */
    memset(buf->map, 0, buf->size);

    return 0;

err_fb:
    drmModeRmFB(fd, buf->fb);
err_destroy:
    memset(&dreq, 0, sizeof(dreq));
    dreq.handle = buf->handle;
    drmIoctl(fd, DRM_IOCTL_MODE_DESTROY_DUMB, &dreq);
    return ret;
}

static uint8_t next_color(bool *up, uint8_t cur, unsigned int mod)
{
    uint8_t next;

    next = cur + (*up ? 1 : -1) * (rand() % mod);
    if ((*up && next < cur) || (!*up && next > cur)) {
        *up = !*up;
        next = cur;
    }

    return next;
}

static void modeset_draw(int fd)
{
    uint8_t r, g, b;
    bool r_up, g_up, b_up;
    unsigned int i, j, k, off;
    struct modeset_dev *iter;
	struct modeset_buf *buf;
    int ret;

    srand(time(NULL));
    r = rand() % 0xff;
    g = rand() % 0xff;
    b = rand() % 0xff;
    r_up = g_up = b_up = true;


    for (i = 0; i < 50; ++i) {        
        r = next_color(&r_up, r, 20);
		g = next_color(&g_up, g, 10);
		b = next_color(&b_up, b, 5);

        /* perform actual modesetting on each found connector+CRTC */
		for (iter = modeset_list; iter; iter = iter->next) {
			buf = &iter->bufs[iter->front_buf ^ 1];
			for (j = 0; j < buf->height; ++j) {
				for (k = 0; k < buf->width; ++k) {
					off = buf->stride * j + k * 4;
					*(uint32_t*)&buf->map[off] =
						     (r << 16) | (g << 8) | b;
				}
			}

			ret = drmModeSetCrtc(fd, iter->crtc, buf->fb, 0, 0,
					     &iter->conn, 1, &iter->mode);
			if (ret)
				fprintf(stderr, "cannot flip CRTC for connector %u (%d): %m\n",
					iter->conn, errno);
			else
				iter->front_buf ^= 1;
		}
        usleep(100000);
    }
}

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

    /* iterate all connectors */
    for (int i = 0; i < res->count_connectors; ++i) {
        drmModeConnector *conn;
        /* get information for each connector */
        conn = drmModeGetConnector(fd, res->connectors[i]);
        if (!conn) {
            printf("cannot retrieve DRM connector %u:%u (%d): %m\n",
            i, res->connectors[i], errno);
            continue;
        }

        printf("get connector correct!\n");
        
        /* check if a monitor is connected */
        if (conn->connection != DRM_MODE_CONNECTED) {
            fprintf(stderr, "ignoring unused connector %u\n",
            conn->connector_id);
            continue;
        }

        /* check if there is at least one valid mode */
        if (conn->count_modes == 0) {
            fprintf(stderr, "no valid mode for connector %u\n",
            conn->connector_id);
            continue;
        }

        /* create a device structure */
        dev = malloc(sizeof(*dev));
        memset(dev, 0, sizeof(*dev));
        dev->conn = conn->connector_id;

        /* copy the mode information into our device structure */
        memcpy(&dev->mode, &conn->modes[0], sizeof(dev->mode));
        dev->bufs[0].width = conn->modes[0].hdisplay;
        dev->bufs[0].height = conn->modes[0].vdisplay;
        dev->bufs[1].width = conn->modes[0].hdisplay;
        dev->bufs[1].height = conn->modes[0].vdisplay;
        fprintf(stderr, "mode for connector %u is %ux%u\n",
		    conn->connector_id, dev->bufs[0].width, dev->bufs[0].height);

        /* find a crtc for this connector */
        ret = modeset_find_crtc(fd, res, conn, dev);
        if (ret) {
            fprintf(stderr, "no valid crtc for connector %u\n",
            conn->connector_id);
            return ret;
        }

        printf("get crtc correct!\n");
        printf("conn_id: %d, crtc_id: %d\n", conn->connector_id, dev->crtc);

        /* create a framebuffer for this CRTC */
        ret = modeset_create_fb(fd, &dev->bufs[0]);
        if (ret) {
            fprintf(stderr, "cannot create framebuffer for connector %u\n",
            conn->connector_id);
            return ret;
        }
        printf("create buffer1 correct!\n");

        /* create a framebuffer for this CRTC */
        ret = modeset_create_fb(fd, &dev->bufs[1]);
        if (ret) {
            fprintf(stderr, "cannot create framebuffer for connector %u\n",
            conn->connector_id);
            return ret;
        }
        printf("create buffer2 correct!\n");

        /* free connector data and link device into global list */
        drmModeFreeConnector(conn);
        dev->next = modeset_list;
        modeset_list = dev;
    }

    /* perform actual modesetting on each found connector+CRTC */
	for (iter = modeset_list; iter; iter = iter->next) {
		iter->saved_crtc = drmModeGetCrtc(fd, iter->crtc);
		buf = &iter->bufs[iter->front_buf];
		ret = drmModeSetCrtc(fd, iter->crtc, buf->fb, 0, 0,
				     &iter->conn, 1, &iter->mode);
		if (ret)
			fprintf(stderr, "cannot set CRTC for connector %u (%d): %m\n",
				iter->conn, errno);
	}

    /* draw some colors for 5seconds */
    modeset_draw(fd);

    drmModeFreeResources(res);
    close(fd);
    return 0;
}
