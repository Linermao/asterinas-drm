#include <assert.h>
#include <ctype.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <inttypes.h>
#include <unistd.h>
#include <string.h>
#include <strings.h>
#include <errno.h>
#include <poll.h>
#include <sys/time.h>
#if HAVE_SYS_SELECT_H
#include <sys/select.h>
#endif
#include <math.h>

#include "xf86drm.h"
#include "xf86drmMode.h"
#include "drm_fourcc.h"

#include "../util/format.h"
#include "../util/kms.h"
#include "../util/pattern.h"

#ifndef UTIL_COMMON_H
#define UTIL_COMMON_H

#ifndef ARRAY_SIZE
#define ARRAY_SIZE(a) (sizeof(a) / sizeof((a)[0]))
#endif

#endif /* UTIL_COMMON_H */

struct crtc {
	drmModeCrtc *crtc;
	drmModeObjectProperties *props;
	drmModePropertyRes **props_info;
	drmModeModeInfo *mode;
};

struct encoder {
	drmModeEncoder *encoder;
};

struct connector {
	drmModeConnector *connector;
	drmModeObjectProperties *props;
	drmModePropertyRes **props_info;
	char *name;
};

struct fb {
	drmModeFB *fb;
};

struct plane {
	drmModePlane *plane;
	drmModeObjectProperties *props;
	drmModePropertyRes **props_info;
};

struct resources {
	struct crtc *crtcs;
	int count_crtcs;
	struct encoder *encoders;
	int count_encoders;
	struct connector *connectors;
	int count_connectors;
	struct fb *fbs;
	int count_fbs;
	struct plane *planes;
	uint32_t count_planes;
};

struct device {
	int fd;

	struct resources *resources;

	struct {
		unsigned int width;
		unsigned int height;

		unsigned int fb_id;
		struct bo *bo;
		struct bo *cursor_bo;
	} mode;

	int use_atomic;
	drmModeAtomicReq *req;
};

/* -----------------------------------------------------------------------------
 * Pipes and planes
 */

/*
 * Mode setting with the kernel interfaces is a bit of a chore.
 * First you have to find the connector in question and make sure the
 * requested mode is available.
 * Then you need to find the encoder attached to that connector so you
 * can bind it with a free crtc.
 */
struct pipe_arg {
	const char **cons;
	uint32_t *con_ids;
	unsigned int num_cons;
	uint32_t crtc_id;
	char mode_str[64];
	char format_str[5];
	float vrefresh;
	unsigned int fourcc;
	drmModeModeInfo *mode;
	struct crtc *crtc;
	unsigned int fb_id[2], current_fb_id;
	struct timeval start;

	int swap_count;
};

struct plane_arg {
	uint32_t plane_id;  /* the id of plane to use */
	uint32_t crtc_id;  /* the id of CRTC to bind to */
	bool has_position;
	int32_t x, y;
	uint32_t w, h;
	double scale;
	unsigned int fb_id;
	unsigned int old_fb_id;
	struct bo *bo;
	struct bo *old_bo;
	char format_str[5]; /* need to leave room for terminating \0 */
	unsigned int fourcc;
};

/* -----------------------------------------------------------------------------
 * Properties
 */

struct property_arg {
	uint32_t obj_id;
	uint32_t obj_type;
	char name[DRM_PROP_NAME_LEN+1];
	uint32_t prop_id;
	uint64_t value;
	bool optional;
};

void dump_fourcc(uint32_t fourcc);
void dump_encoders(struct device *dev);
void dump_mode(drmModeModeInfo *mode, int index);
void dump_blob(struct device *dev, uint32_t blob_id);
void dump_in_formats(struct device *dev, uint32_t blob_id);
void dump_prop(struct device *dev, drmModePropertyPtr prop,
		      uint32_t prop_id, uint64_t value);
void dump_connectors(struct device *dev);
void dump_crtcs(struct device *dev);
void dump_framebuffers(struct device *dev);
void dump_planes(struct device *dev);
void free_resources(struct resources *res);

struct resources *get_resources(struct device *dev);
struct crtc *get_crtc_by_id(struct device *dev, uint32_t id);
uint32_t get_crtc_mask(struct device *dev, struct crtc *crtc);
drmModeConnector *get_connector_by_name(struct device *dev, const char *name);
drmModeConnector *get_connector_by_id(struct device *dev, uint32_t id);
drmModeEncoder *get_encoder_by_id(struct device *dev, uint32_t id);

drmModeModeInfo *
connector_find_mode(struct device *dev, uint32_t con_id, const char *mode_str,
	const float vrefresh);
struct crtc *pipe_find_crtc(struct device *dev, struct pipe_arg *pipe);
int pipe_find_crtc_and_mode(struct device *dev, struct pipe_arg *pipe);
bool set_property(struct device *dev, struct property_arg *p);

void
page_flip_handler(int fd, unsigned int frame,
		  unsigned int sec, unsigned int usec, void *data);
bool format_support(const drmModePlanePtr ovr, uint32_t fmt);

void add_property(struct device *dev, uint32_t obj_id,
			       const char *name, uint64_t value);
bool add_property_optional(struct device *dev, uint32_t obj_id,
				  const char *name, uint64_t value);

void set_gamma(struct device *dev, unsigned crtc_id, unsigned fourcc);
int
bo_fb_create(int fd, unsigned int fourcc, const uint32_t w, const uint32_t h,
             enum util_fill_pattern pat, struct bo **out_bo, unsigned int *out_fb_id);

int atomic_set_plane(struct device *dev, struct plane_arg *p,
							int pattern, bool update);
int set_plane(struct device *dev, struct plane_arg *p);
void atomic_set_planes(struct device *dev, struct plane_arg *p,
			      unsigned int count, bool update);
void
atomic_test_page_flip(struct device *dev, struct pipe_arg *pipe_args,
              struct plane_arg *plane_args, unsigned int plane_count);
void atomic_clear_planes(struct device *dev, struct plane_arg *p, unsigned int count);
void atomic_clear_FB(struct device *dev, struct plane_arg *p, unsigned int count);
void clear_planes(struct device *dev, struct plane_arg *p, unsigned int count);
int pipe_resolve_connectors(struct device *dev, struct pipe_arg *pipe);
int pipe_attempt_connector(struct device *dev, drmModeConnector *con,
		struct pipe_arg *pipe);
int pipe_find_preferred(struct device *dev, struct pipe_arg **out_pipes);
struct plane *get_primary_plane_by_crtc(struct device *dev, struct crtc *crtc);
void set_mode(struct device *dev, struct pipe_arg *pipes, unsigned int count);
void atomic_clear_mode(struct device *dev, struct pipe_arg *pipes, unsigned int count);
void clear_mode(struct device *dev);
void set_planes(struct device *dev, struct plane_arg *p, unsigned int count);
void set_cursors(struct device *dev, struct pipe_arg *pipes, unsigned int count);
void clear_cursors(struct device *dev);
void test_page_flip(struct device *dev, struct pipe_arg *pipes, unsigned int count);
int parse_connector(struct pipe_arg *pipe, const char *arg);
int parse_plane(struct plane_arg *plane, const char *p);
int parse_property(struct property_arg *p, const char *arg);
void parse_fill_patterns(char *arg);

            

