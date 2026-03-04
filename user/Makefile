FLAGS=`pkg-config --cflags --libs libdrm`
FLAGS+=-Wall -O0 -g
FLAGS+=-D_FILE_OFFSET_BITS=64

# 源码路径
MODETEST_SRCS := ./modetest/modetest.c
UTIL_SRCS     := ./util/common.c ./util/format.c ./util/kms.c ./util/pattern.c ./util/buffers.c ./util/cursor.c

all:
	@echo "==> Building simpletest..."
	gcc -static ./simpletest/fb_test.c $(UTIL_SRCS) -o build/fb_test $(FLAGS)
	gcc -static ./simpletest/get_resource.c $(UTIL_SRCS) -o build/get_resource $(FLAGS)
	gcc -static ./simpletest/double_framebuffer.c $(UTIL_SRCS) -o build/double_framebuffer $(FLAGS)
	gcc -static ./simpletest/resources_test.c $(UTIL_SRCS) -o build/resources_test $(FLAGS)
	gcc -static ./simpletest/modeset.c $(UTIL_SRCS) -o build/modeset $(FLAGS)
	gcc -static ./page_flip/page_flip.c $(UTIL_SRCS) -o build/page_flip $(FLAGS)
	gcc -static ./page_flip/test_vblank.c $(UTIL_SRCS) -o build/test_vblank $(FLAGS)
	gcc -static ./page_flip/vsync.c $(UTIL_SRCS) -o build/vsync $(FLAGS)

# 	@echo "==> Building modetest..."
# 	gcc -static $(MODETEST_SRCS) $(UTIL_SRCS) -o build/modetest $(FLAGS)

	@echo "==> Build finished successfully."
