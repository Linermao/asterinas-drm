// SPDX-License-Identifier: MPL-2.0

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <linux/input.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <unistd.h>

#include "../common/test.h"

#define INPUT_DIR "/dev/input"
#define MAX_EVDEV_DEVICES 16

static int evdev_fds[MAX_EVDEV_DEVICES];
static size_t evdev_count = 0;

static void cleanup_open_fds(DIR *dir)
{
	if (dir) {
		closedir(dir);
	}
	for (size_t i = 0; i < evdev_count; ++i) {
		close(evdev_fds[i]);
	}
}

FN_SETUP(open_evdev)
{
	DIR *dir = opendir(INPUT_DIR);
	if (!dir) {
		if (errno == ENOENT || errno == ENODEV || errno == ENXIO) {
			fprintf(stderr, "evdev tests skipped: %s (%s)\n",
				INPUT_DIR, strerror(errno));
			exit(EXIT_SUCCESS);
		}
		fprintf(stderr,
			"fatal error: setup_open_evdev: opendir('%s') failed: %s\n",
			INPUT_DIR, strerror(errno));
		exit(EXIT_FAILURE);
	}

	struct dirent *entry;
	while ((entry = readdir(dir)) != NULL) {
		if (strncmp(entry->d_name, "event", 5) != 0) {
			continue;
		}

		if (evdev_count >= MAX_EVDEV_DEVICES) {
			cleanup_open_fds(dir);
			fprintf(stderr,
				"fatal error: setup_open_evdev: too many event devices (max %d)\n",
				MAX_EVDEV_DEVICES);
			exit(EXIT_FAILURE);
		}

		char path[PATH_MAX];
		int len = snprintf(path, sizeof(path), "%s/%s", INPUT_DIR,
				   entry->d_name);
		if (len < 0 || len >= (int)sizeof(path)) {
			cleanup_open_fds(dir);
			fprintf(stderr,
				"fatal error: setup_open_evdev: path too long\n");
			exit(EXIT_FAILURE);
		}

		int fd = open(path, O_RDONLY);
		if (fd < 0) {
			int saved = errno;
			cleanup_open_fds(dir);
			fprintf(stderr,
				"fatal error: setup_open_evdev: open('%s') failed: %s\n",
				path, strerror(saved));
			exit(EXIT_FAILURE);
		}

		evdev_fds[evdev_count++] = fd;
	}

	closedir(dir);

	if (evdev_count == 0) {
		fprintf(stderr,
			"evdev tests skipped: no event devices found in %s\n",
			INPUT_DIR);
		exit(EXIT_SUCCESS);
	}
}
END_SETUP()

FN_TEST(open_close)
{
	TEST_RES(evdev_count > 0, _ret == 1);
	for (size_t i = 0; i < evdev_count; ++i) {
		TEST_RES(evdev_fds[i] >= 0, _ret == 1);
	}
}
END_TEST()

static void read_sysfs(const char *path, char *buffer, size_t size)
{
	int fd = CHECK(open(path, O_RDONLY));
	ssize_t len = CHECK(read(fd, buffer, size - 1));
	buffer[len] = '\0';
	CHECK(close(fd));
}

/* Decode one bit from the high-word-first bitmap used in input uevents. */
static int uevent_has_bit(const char *uevent, const char *field, unsigned bit)
{
	const char *value = uevent;
	size_t field_len = strlen(field);
	while (strncmp(value, field, field_len) != 0) {
		value = strchr(value, '\n');
		if (!value || !*++value) {
			return -1;
		}
	}
	value += field_len;
	const char *end = strchr(value, '\n');
	if (!end) {
		return -1;
	}
	unsigned word_index = bit / (sizeof(unsigned long) * 8);
	while (end > value) {
		const char *word = end;
		while (word > value && word[-1] != ' ') {
			--word;
		}
		if (word_index == 0) {
			unsigned long bits = strtoul(word, NULL, 16);
			return !!(bits & (1UL << (bit % (sizeof(bits) * 8))));
		}
		--word_index;
		end = word > value ? word - 1 : value;
	}
	return 0;
}

static int uevent_matches_bits(const char *uevent, const char *field,
			       const unsigned char *bits, unsigned max_bit)
{
	for (unsigned bit = 0; bit <= max_bit; ++bit) {
		if (uevent_has_bit(uevent, field, bit) !=
		    !!(bits[bit / 8] & (1U << (bit % 8)))) {
			return 0;
		}
	}
	return 1;
}

FN_TEST(sysfs_discovery)
{
	for (size_t i = 0; i < evdev_count; ++i) {
		struct stat device_stat;
		CHECK(fstat(evdev_fds[i], &device_stat));
		char path[PATH_MAX];
		char event_uevent[512];
		snprintf(path, sizeof(path), "/sys/dev/char/%u:%u/uevent",
			 major(device_stat.st_rdev), minor(device_stat.st_rdev));
		read_sysfs(path, event_uevent, sizeof(event_uevent));
		const char *devname = strstr(event_uevent, "DEVNAME=input/event");
		TEST_RES(devname != NULL, _ret == 1);

		/* Both indexes must resolve to the same published event device. */
		struct stat dev_index_stat, class_stat;
		snprintf(path, sizeof(path), "/sys/dev/char/%u:%u",
			 major(device_stat.st_rdev), minor(device_stat.st_rdev));
		CHECK(stat(path, &dev_index_stat));
		snprintf(path, sizeof(path), "/sys/class/input/event%u",
			 minor(device_stat.st_rdev));
		CHECK(stat(path, &class_stat));
		TEST_RES(dev_index_stat.st_ino == class_stat.st_ino &&
				 dev_index_stat.st_dev == class_stat.st_dev,
			 _ret == 1);

		/* These are the parent capabilities used by libudev-zero input_id. */
		char input_uevent[2048];
		snprintf(path, sizeof(path),
			 "/sys/class/input/event%u/device/uevent",
			 minor(device_stat.st_rdev));
		read_sysfs(path, input_uevent, sizeof(input_uevent));
		unsigned char event_bits[(EV_MAX + 8) / 8] = { 0 };
		unsigned char key_bits[(KEY_MAX + 8) / 8] = { 0 };
		unsigned char rel_bits[(REL_MAX + 8) / 8] = { 0 };
		CHECK(ioctl(evdev_fds[i], EVIOCGBIT(0, sizeof(event_bits)),
			    event_bits));
		CHECK(ioctl(evdev_fds[i], EVIOCGBIT(EV_KEY, sizeof(key_bits)),
			    key_bits));
		CHECK(ioctl(evdev_fds[i], EVIOCGBIT(EV_REL, sizeof(rel_bits)),
			    rel_bits));
		TEST_RES(uevent_matches_bits(input_uevent, "EV=", event_bits,
					     EV_MAX),
			 _ret == 1);
		TEST_RES(uevent_matches_bits(input_uevent, "KEY=", key_bits,
					     KEY_MAX),
			 _ret == 1);
		TEST_RES(uevent_matches_bits(input_uevent, "REL=", rel_bits,
					     REL_MAX),
			 _ret == 1);

		unsigned char props[4];
		memset(props, 0xff, sizeof(props));
		TEST_SUCC(ioctl(evdev_fds[i], EVIOCGPROP(sizeof(props)), props));
		TEST_RES(memcmp(props, (unsigned char[4]){ 0 }, sizeof(props)),
			 _ret == 0);
		TEST_RES(uevent_has_bit(input_uevent, "PROP=", 0), _ret == 0);
	}
}
END_TEST()

FN_SETUP(close_evdev)
{
	for (size_t i = 0; i < evdev_count; ++i) {
		CHECK(close(evdev_fds[i]));
	}
}
END_SETUP()
