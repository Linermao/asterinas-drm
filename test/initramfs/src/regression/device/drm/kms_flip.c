// SPDX-License-Identifier: MPL-2.0

#include "../../common/test.h"
#include "common.h"
#include <drm/drm_fourcc.h>
#include <poll.h>
#include <sys/mman.h>
#include <sys/wait.h>

static int card_fd;
static drmModeResPtr resources;
static drmModeConnectorPtr connector;
static drmModeCrtcPtr saved_crtc;
static uint32_t crtc_id;
static uint32_t dpms_id;

struct scanout_buffer {
	struct drm_mode_create_dumb dumb;
	uint32_t fb_id;
	void *mapping;
};

static struct scanout_buffer buffers[2];

static void create_buffer(struct scanout_buffer *buffer, uint32_t color)
{
	buffer->dumb.width = connector->modes[0].hdisplay;
	buffer->dumb.height = connector->modes[0].vdisplay;
	buffer->dumb.bpp = 32;
	CHECK(ioctl(card_fd, DRM_IOCTL_MODE_CREATE_DUMB, &buffer->dumb));
	uint32_t handles[4] = { buffer->dumb.handle };
	uint32_t pitches[4] = { buffer->dumb.pitch };
	uint32_t offsets[4] = { 0 };
	CHECK(drmModeAddFB2(card_fd, buffer->dumb.width, buffer->dumb.height,
			    DRM_FORMAT_XRGB8888, handles, pitches, offsets,
			    &buffer->fb_id, 0));
	struct drm_mode_map_dumb map = { .handle = buffer->dumb.handle };
	CHECK(ioctl(card_fd, DRM_IOCTL_MODE_MAP_DUMB, &map));
	buffer->mapping =
		CHECK_WITH(mmap(NULL, buffer->dumb.size, PROT_READ | PROT_WRITE,
				MAP_SHARED, card_fd, map.offset),
			   _ret != MAP_FAILED);
	for (uint32_t y = 0; y < buffer->dumb.height; ++y) {
		uint32_t *row = (uint32_t *)((char *)buffer->mapping +
					     y * buffer->dumb.pitch);
		for (uint32_t x = 0; x < buffer->dumb.width; ++x) {
			row[x] = color;
		}
	}
}

FN_SETUP(open_display)
{
	card_fd = open_drm_node_or_skip(DRM_CARD_DEVICE);
	CHECK(fcntl(card_fd, F_SETFL, O_NONBLOCK));
	resources = get_kms_resources_or_skip(card_fd);
	CHECK_WITH(resources->count_crtcs, _ret > 0);
	for (int i = 0; i < resources->count_connectors; ++i) {
		connector = CHECK_WITH(
			drmModeGetConnector(card_fd, resources->connectors[i]),
			_ret != NULL);
		if (connector->connection == DRM_MODE_CONNECTED &&
		    connector->count_modes > 0) {
			break;
		}
		drmModeFreeConnector(connector);
		connector = NULL;
	}
	CHECK_WITH(connector, _ret != NULL);
	crtc_id = resources->crtcs[0];
	saved_crtc = CHECK_WITH(drmModeGetCrtc(card_fd, crtc_id), _ret != NULL);
	CHECK(drmSetMaster(card_fd));
	CHECK(drmModeSetCursor(card_fd, crtc_id, 0, 0, 0));
	for (int i = 0; i < connector->count_props; ++i) {
		drmModePropertyPtr property = CHECK_WITH(
			drmModeGetProperty(card_fd, connector->props[i]),
			_ret != NULL);
		if (strcmp(property->name, "DPMS") == 0) {
			dpms_id = property->prop_id;
		}
		drmModeFreeProperty(property);
	}
	CHECK_WITH(dpms_id, _ret != 0);
}
END_SETUP()

FN_TEST(event_fd_is_idle_before_submission)
{
	struct pollfd poll_fd = { .fd = card_fd, .events = POLLIN };
	char events[128];
	TEST_RES(poll(&poll_fd, 1, 0), _ret == 0);
	TEST_ERRNO(read(card_fd, events, sizeof(events)), EAGAIN);
}
END_TEST()

FN_TEST(dpms_reports_current_value_and_rejects_invalid_values)
{
	TEST_ERRNO(drmModeConnectorSetProperty(card_fd, connector->connector_id,
					       dpms_id, 4),
		   EINVAL);
	CHECK(drmModeConnectorSetProperty(card_fd, connector->connector_id,
					  dpms_id, DRM_MODE_DPMS_OFF));
	drmModeObjectPropertiesPtr properties = CHECK_WITH(
		drmModeObjectGetProperties(card_fd, connector->connector_id,
					   DRM_MODE_OBJECT_CONNECTOR),
		_ret != NULL);
	int found = 0;
	for (uint32_t i = 0; i < properties->count_props; ++i) {
		if (properties->props[i] == dpms_id) {
			TEST_RES(properties->prop_values[i],
				 _ret == DRM_MODE_DPMS_OFF);
			found = 1;
		}
	}
	TEST_RES(found, _ret == 1);
	drmModeFreeObjectProperties(properties);
	CHECK(drmModeConnectorSetProperty(card_fd, connector->connector_id,
					  dpms_id, DRM_MODE_DPMS_ON));
}
END_TEST()

FN_TEST(page_flip_delivers_a_complete_event_and_closefb_keeps_scanout)
{
	create_buffer(&buffers[0], 0x00ff0000);
	create_buffer(&buffers[1], 0x0000ff00);
	CHECK(drmModeSetCrtc(card_fd, crtc_id, buffers[0].fb_id, 0, 0,
			     &connector->connector_id, 1,
			     &connector->modes[0]));
	struct drm_mode_crtc_page_flip flip = {
		.crtc_id = crtc_id,
		.fb_id = buffers[1].fb_id,
		.flags = DRM_MODE_PAGE_FLIP_EVENT,
		.user_data = UINT64_C(0x123456789abcdef0),
	};
	flip.reserved = 1;
	TEST_ERRNO(ioctl(card_fd, DRM_IOCTL_MODE_PAGE_FLIP, &flip), EINVAL);
	flip.reserved = 0;
	CHECK(ioctl(card_fd, DRM_IOCTL_MODE_PAGE_FLIP, &flip));
	struct pollfd poll_fd = { .fd = card_fd, .events = POLLIN };
	TEST_RES(poll(&poll_fd, 1, 3000), _ret == 1);
	struct drm_event_vblank event;
	char short_event[sizeof(event) - 1];
	TEST_RES(read(card_fd, short_event, sizeof(short_event)), _ret == 0);
	TEST_RES(poll(&poll_fd, 1, 0), _ret == 1);
	TEST_RES(read(card_fd, &event, sizeof(event)), _ret == sizeof(event));
	TEST_RES(event.base.type, _ret == DRM_EVENT_FLIP_COMPLETE);
	TEST_RES(event.base.length, _ret == sizeof(event));
	TEST_RES(event.user_data, _ret == flip.user_data);
	TEST_RES(event.crtc_id, _ret == crtc_id);
	TEST_RES(event.tv_usec, _ret < 1000000);
	TEST_RES(poll(&poll_fd, 1, 0), _ret == 0);

	struct drm_mode_closefb close_fb = { .fb_id = buffers[1].fb_id };
	CHECK(ioctl(card_fd, DRM_IOCTL_MODE_CLOSEFB, &close_fb));
	drmModeCrtcPtr current =
		CHECK_WITH(drmModeGetCrtc(card_fd, crtc_id), _ret != NULL);
	TEST_RES(current->buffer_id, _ret == buffers[1].fb_id);
	drmModeFreeCrtc(current);
	TEST_ERRNO(ioctl(card_fd, DRM_IOCTL_MODE_CLOSEFB, &close_fb), ENOENT);
	buffers[1].fb_id = 0;
}
END_TEST()

FN_TEST(shared_gem_mapping_remains_writable_after_fork)
{
	volatile uint32_t *pixels = buffers[0].mapping;
	pixels[0] = 0x112233;
	pid_t child = CHECK(fork());
	if (child == 0) {
		// A write-protected device PTE must be repaired rather than fault forever.
		alarm(2);
		pixels[0] = 0x445566;
		_exit(pixels[0] == 0x445566 ? 0 : 1);
	}
	int status;
	CHECK(waitpid(child, &status, 0));
	TEST_RES(status, WIFEXITED(_ret) && WEXITSTATUS(_ret) == 0);
	TEST_RES(pixels[0], _ret == 0x445566);
	alarm(2);
	pixels[0] = 0x778899;
	alarm(0);
	TEST_RES(pixels[0], _ret == 0x778899);
}
END_TEST()

FN_TEST(vblank_waits_and_events_use_the_software_counter)
{
	union drm_wait_vblank wait = {
		.request = { .type = _DRM_VBLANK_RELATIVE, .sequence = 1 },
	};
	CHECK(ioctl(card_fd, DRM_IOCTL_WAIT_VBLANK, &wait));
	uint32_t sequence = wait.reply.sequence;
	wait.request.type = _DRM_VBLANK_RELATIVE | _DRM_VBLANK_EVENT;
	wait.request.sequence = 1;
	wait.request.signal = 0x12345;
	CHECK(ioctl(card_fd, DRM_IOCTL_WAIT_VBLANK, &wait));
	struct pollfd poll_fd = { .fd = card_fd, .events = POLLIN };
	TEST_RES(poll(&poll_fd, 1, 3000), _ret == 1);
	struct drm_event_vblank event;
	TEST_RES(read(card_fd, &event, sizeof(event)), _ret == sizeof(event));
	TEST_RES(event.base.type, _ret == DRM_EVENT_VBLANK);
	TEST_RES(event.user_data, _ret == 0x12345);
	TEST_RES(event.crtc_id, _ret == crtc_id);
	TEST_RES((int32_t)(event.sequence - sequence), _ret > 0);
}
END_TEST()

FN_SETUP(close_display)
{
	int restored = 0;
	if (saved_crtc->mode_valid) {
		int result = drmModeSetCrtc(card_fd, crtc_id,
					    saved_crtc->buffer_id,
					    saved_crtc->x, saved_crtc->y,
					    &connector->connector_id, 1,
					    &saved_crtc->mode);
		if (result == 0) {
			restored = 1;
		} else {
			// A stopped compositor may have closed its scanout FB. Replacing
			// that FB releases its last reference, so it cannot be restored.
			int restore_errno = errno;
			CHECK_WITH(restore_errno, _ret == ENOENT);
		}
	}
	if (!restored) {
		CHECK(drmModeSetCrtc(card_fd, crtc_id, 0, 0, 0, NULL, 0, NULL));
	}
	for (size_t i = 0; i < 2; ++i) {
		if (buffers[i].fb_id != 0) {
			CHECK(drmModeRmFB(card_fd, buffers[i].fb_id));
		}
		CHECK(munmap(buffers[i].mapping, buffers[i].dumb.size));
		struct drm_mode_destroy_dumb destroy = {
			.handle = buffers[i].dumb.handle,
		};
		CHECK(ioctl(card_fd, DRM_IOCTL_MODE_DESTROY_DUMB, &destroy));
	}
	drmModeFreeCrtc(saved_crtc);
	drmModeFreeConnector(connector);
	drmModeFreeResources(resources);
	CHECK(close(card_fd));
}
END_SETUP()
