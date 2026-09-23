# SPDX-License-Identifier: MPL-2.0

CC ?= cc
PKG_CONFIG ?= pkg-config
PREFIX ?= /usr/local

CFLAGS += -Wall -Wextra -Werror
CFLAGS += $(shell $(PKG_CONFIG) --cflags libdrm)
LDLIBS += $(shell $(PKG_CONFIG) --libs libdrm)

.PHONY: all install format check clean

all: kms_resources set_crtc

kms_resources: kms_resources.c
	$(CC) $(CFLAGS) $< -o $@ $(LDFLAGS) $(LDLIBS)

set_crtc: set_crtc.c
	$(CC) $(CFLAGS) $< -o $@ $(LDFLAGS) $(LDLIBS)

install: kms_resources set_crtc
	install -Dm755 kms_resources $(DESTDIR)$(PREFIX)/bin/kms_resources
	install -Dm755 set_crtc $(DESTDIR)$(PREFIX)/bin/set_crtc

format:
	clang-format -i kms_resources.c set_crtc.c

check:
	clang-format --dry-run --Werror kms_resources.c set_crtc.c

clean:
	rm -f kms_resources set_crtc
