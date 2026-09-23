/*
 * modeset - DRM Modesetting Example
 *
 * Written 2012 by David Rheinsberg <david.rheinsberg@gmail.com>
 * Dedicated to the Public Domain.
 */

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include <xf86drm.h>
#include <xf86drmMode.h>

struct modeset_dev;
static int modeset_find_crtc(int fd, drmModeRes *res, drmModeConnector *conn,
                             struct modeset_dev *dev);
static int modeset_create_fb(int fd, struct modeset_dev *dev);
static int modeset_setup_dev(int fd, drmModeRes *res, drmModeConnector *conn,
                             struct modeset_dev *dev);
static int modeset_open(int *out, const char *node);
static int modeset_prepare(int fd);
static int modeset_dirty_fb(int fd, struct modeset_dev *dev);
static int modeset_draw(int fd);
static void modeset_cleanup(int fd);

static int modeset_open(int *out, const char *node) {
  int fd, ret;
  uint64_t has_dumb;

  fd = open(node, O_RDWR | O_CLOEXEC);
  if (fd < 0) {
    ret = -errno;
    fprintf(stderr, "cannot open '%s': %m\n", node);
    return ret;
  }

  if (drmGetCap(fd, DRM_CAP_DUMB_BUFFER, &has_dumb) < 0 || !has_dumb) {
    fprintf(stderr, "drm device '%s' does not support dumb buffers\n", node);
    close(fd);
    return -EOPNOTSUPP;
  }

  *out = fd;
  return 0;
}

struct modeset_dev {
  struct modeset_dev *next;

  uint32_t width;
  uint32_t height;
  uint32_t stride;
  uint32_t size;
  uint32_t handle;
  uint8_t *map;

  drmModeModeInfo mode;
  uint32_t fb;
  uint32_t conn;
  uint32_t crtc;
  drmModeCrtc *saved_crtc;
};

static struct modeset_dev *modeset_list = NULL;

static int modeset_prepare(int fd) {
  drmModeRes *res;
  drmModeConnector *conn;
  int i;
  struct modeset_dev *dev;
  int ret;

  /* retrieve resources */
  res = drmModeGetResources(fd);
  if (!res) {
    fprintf(stderr, "cannot retrieve DRM resources (%d): %m\n", errno);
    return -errno;
  }

  /* iterate all connectors */
  for (i = 0; i < res->count_connectors; ++i) {
    /* get information for each connector */
    conn = drmModeGetConnector(fd, res->connectors[i]);
    if (!conn) {
      fprintf(stderr, "cannot retrieve DRM connector %d:%u (%d): %m\n", i,
              res->connectors[i], errno);
      continue;
    }

    /* create a device structure */
    dev = malloc(sizeof(*dev));
    memset(dev, 0, sizeof(*dev));
    dev->conn = conn->connector_id;

    /* call helper function to prepare this connector */
    ret = modeset_setup_dev(fd, res, conn, dev);
    if (ret) {
      if (ret != -ENOENT) {
        errno = -ret;
        fprintf(stderr, "cannot setup device for connector %d:%u (%d): %m\n", i,
                res->connectors[i], errno);
      }
      free(dev);
      drmModeFreeConnector(conn);
      continue;
    }

    /* free connector data and link device into global list */
    drmModeFreeConnector(conn);
    dev->next = modeset_list;
    modeset_list = dev;
  }

  /* free resources again */
  drmModeFreeResources(res);
  return 0;
}

static int modeset_setup_dev(int fd, drmModeRes *res, drmModeConnector *conn,
                             struct modeset_dev *dev) {
  int ret;

  /* check if a monitor is connected */
  if (conn->connection != DRM_MODE_CONNECTED) {
    fprintf(stderr, "ignoring unused connector %u\n", conn->connector_id);
    return -ENOENT;
  }

  /* check if there is at least one valid mode */
  if (conn->count_modes == 0) {
    fprintf(stderr, "no valid mode for connector %u\n", conn->connector_id);
    return -EFAULT;
  }

  /* copy the mode information into our device structure */
  memcpy(&dev->mode, &conn->modes[0], sizeof(dev->mode));
  dev->width = conn->modes[0].hdisplay;
  dev->height = conn->modes[0].vdisplay;
  fprintf(stderr, "mode for connector %u is %ux%u\n", conn->connector_id,
          dev->width, dev->height);

  /* find a crtc for this connector */
  ret = modeset_find_crtc(fd, res, conn, dev);
  if (ret) {
    fprintf(stderr, "no valid crtc for connector %u\n", conn->connector_id);
    return ret;
  }

  /* create a framebuffer for this CRTC */
  ret = modeset_create_fb(fd, dev);
  if (ret) {
    fprintf(stderr, "cannot create framebuffer for connector %u\n",
            conn->connector_id);
    return ret;
  }

  return 0;
}

static int modeset_find_crtc(int fd, drmModeRes *res, drmModeConnector *conn,
                             struct modeset_dev *dev) {
  drmModeEncoder *enc;
  int i, j;
  uint32_t crtc;
  bool crtc_used;
  struct modeset_dev *iter;

  /* first try the currently conected encoder+crtc */
  if (conn->encoder_id)
    enc = drmModeGetEncoder(fd, conn->encoder_id);
  else
    enc = NULL;

  if (enc) {
    if (enc->crtc_id) {
      crtc = enc->crtc_id;
      crtc_used = false;
      for (iter = modeset_list; iter; iter = iter->next) {
        if (iter->crtc == crtc) {
          crtc_used = true;
          break;
        }
      }

      if (!crtc_used) {
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
      fprintf(stderr, "cannot retrieve encoder %d:%u (%d): %m\n", i,
              conn->encoders[i], errno);
      continue;
    }

    /* iterate all global CRTCs */
    for (j = 0; j < res->count_crtcs; ++j) {
      /* check whether this CRTC works with the encoder */
      if (!(enc->possible_crtcs & (1U << j)))
        continue;

      /* check that no other device already uses this CRTC */
      crtc = res->crtcs[j];
      crtc_used = false;
      for (iter = modeset_list; iter; iter = iter->next) {
        if (iter->crtc == crtc) {
          crtc_used = true;
          break;
        }
      }

      /* we have found a CRTC, so save it and return */
      if (!crtc_used) {
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

static int modeset_create_fb(int fd, struct modeset_dev *dev) {
  struct drm_mode_create_dumb creq;
  struct drm_mode_destroy_dumb dreq;
  struct drm_mode_map_dumb mreq;
  int ret;

  /* create dumb buffer */
  memset(&creq, 0, sizeof(creq));
  creq.width = dev->width;
  creq.height = dev->height;
  creq.bpp = 32;
  ret = drmIoctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &creq);
  if (ret < 0) {
    fprintf(stderr, "cannot create dumb buffer (%d): %m\n", errno);
    return -errno;
  }
  dev->stride = creq.pitch;
  dev->size = creq.size;
  dev->handle = creq.handle;

  /* create framebuffer object for the dumb-buffer */
  ret = drmModeAddFB(fd, dev->width, dev->height, 24, 32, dev->stride,
                     dev->handle, &dev->fb);
  if (ret) {
    fprintf(stderr, "cannot create framebuffer (%d): %m\n", errno);
    ret = -errno;
    goto err_destroy;
  }

  /* prepare buffer for memory mapping */
  memset(&mreq, 0, sizeof(mreq));
  mreq.handle = dev->handle;
  ret = drmIoctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &mreq);
  if (ret) {
    fprintf(stderr, "cannot map dumb buffer (%d): %m\n", errno);
    ret = -errno;
    goto err_fb;
  }

  /* perform actual memory mapping */
  dev->map =
      mmap(0, dev->size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, mreq.offset);
  if (dev->map == MAP_FAILED) {
    fprintf(stderr, "cannot mmap dumb buffer (%d): %m\n", errno);
    ret = -errno;
    goto err_fb;
  }

  /* clear the framebuffer to 0 */
  memset(dev->map, 0, dev->size);

  return 0;

err_fb:
  drmModeRmFB(fd, dev->fb);
err_destroy:
  memset(&dreq, 0, sizeof(dreq));
  dreq.handle = dev->handle;
  drmIoctl(fd, DRM_IOCTL_MODE_DESTROY_DUMB, &dreq);
  return ret;
}

int main(int argc, char **argv) {
  int ret, fd = -1;
  const char *card;
  struct modeset_dev *iter;

  /* check which DRM device to open */
  if (argc > 1)
    card = argv[1];
  else
    card = "/dev/dri/card0";

  fprintf(stderr, "using card '%s'\n", card);

  /* open the DRM device */
  ret = modeset_open(&fd, card);
  if (ret)
    goto out_return;

  /* prepare all connectors and CRTCs */
  ret = modeset_prepare(fd);
  if (ret)
    goto out_close;

  /* perform actual modesetting on each found connector+CRTC */
  for (iter = modeset_list; iter; iter = iter->next) {
    iter->saved_crtc = drmModeGetCrtc(fd, iter->crtc);
    ret = drmModeSetCrtc(fd, iter->crtc, iter->fb, 0, 0, &iter->conn, 1,
                         &iter->mode);
    if (ret)
      fprintf(stderr, "cannot set CRTC for connector %u (%d): %m\n", iter->conn,
              errno);
  }

  /* draw some colors for 5seconds */
  ret = modeset_draw(fd);

  /* cleanup everything */
  modeset_cleanup(fd);

out_close:
  close(fd);
out_return:
  if (ret) {
    errno = -ret;
    fprintf(stderr, "modeset failed with error %d: %m\n", errno);
  } else {
    fprintf(stderr, "exiting\n");
  }
  return ret;
}

static uint8_t next_color(bool *up, uint8_t cur, unsigned int mod) {
  uint8_t next;

  next = cur + (*up ? 1 : -1) * (rand() % mod);
  if ((*up && next < cur) || (!*up && next > cur)) {
    *up = !*up;
    next = cur;
  }

  return next;
}

static int modeset_dirty_fb(int fd, struct modeset_dev *dev) {
  struct drm_mode_fb_dirty_cmd dirty;

  memset(&dirty, 0, sizeof(dirty));
  dirty.fb_id = dev->fb;

  if (drmIoctl(fd, DRM_IOCTL_MODE_DIRTYFB, &dirty) < 0) {
    int ret = -errno;

    fprintf(stderr, "cannot refresh framebuffer %u (%d): %m\n", dev->fb, errno);
    return ret;
  }

  return 0;
}

static int modeset_draw(int fd) {
  uint8_t r, g, b;
  bool r_up, g_up, b_up;
  unsigned int i, j, k, off;
  struct modeset_dev *iter;
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

    for (iter = modeset_list; iter; iter = iter->next) {
      for (j = 0; j < iter->height; ++j) {
        for (k = 0; k < iter->width; ++k) {
          off = iter->stride * j + k * 4;
          *(uint32_t *)&iter->map[off] = (r << 16) | (g << 8) | b;
        }
      }

      ret = modeset_dirty_fb(fd, iter);
      if (ret)
        return ret;
    }

    usleep(100000);
  }

  return 0;
}

static void modeset_cleanup(int fd) {
  struct modeset_dev *iter;
  struct drm_mode_destroy_dumb dreq;
  int ret;

  while (modeset_list) {
    /* remove from global list */
    iter = modeset_list;
    modeset_list = iter->next;

    /* restore saved CRTC configuration, or disable an initially idle CRTC */
    if (iter->saved_crtc && iter->saved_crtc->mode_valid) {
      ret = drmModeSetCrtc(fd, iter->saved_crtc->crtc_id,
                           iter->saved_crtc->buffer_id, iter->saved_crtc->x,
                           iter->saved_crtc->y, &iter->conn, 1,
                           &iter->saved_crtc->mode);
    } else {
      ret = drmModeSetCrtc(fd, iter->crtc, 0, 0, 0, NULL, 0, NULL);
    }
    if (ret)
      fprintf(stderr, "cannot restore CRTC %u (%d): %m\n", iter->crtc, errno);
    if (iter->saved_crtc)
      drmModeFreeCrtc(iter->saved_crtc);

    /* unmap buffer */
    munmap(iter->map, iter->size);

    /* delete framebuffer */
    drmModeRmFB(fd, iter->fb);

    /* delete dumb buffer */
    memset(&dreq, 0, sizeof(dreq));
    dreq.handle = iter->handle;
    drmIoctl(fd, DRM_IOCTL_MODE_DESTROY_DUMB, &dreq);

    /* free allocated memory */
    free(iter);
  }
}
