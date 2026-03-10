#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <stdbool.h>
#include <unistd.h>
#include <time.h>
#include <fcntl.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <linux/fb.h>

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

// >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>
// 绘制函数：向 framebuffer 写入颜色
// >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>
static void modeset_draw(uint8_t *fbmem,
                         struct fb_var_screeninfo *vinfo,
                         struct fb_fix_screeninfo *finfo)
{
    uint8_t r, g, b;
    bool r_up, g_up, b_up;
    unsigned int i, x, y;
    unsigned int bytes_per_pixel = vinfo->bits_per_pixel / 8;

    srand(time(NULL));
    r = rand() % 0xff;
    g = rand() % 0xff;
    b = rand() % 0xff;
    r_up = g_up = b_up = true;

    printf("Start drawing...\n");

    for (i = 0; i < 200; i++) {
        r = next_color(&r_up, r, 20);
        g = next_color(&g_up, g, 10);
        b = next_color(&b_up, b, 5);

        // ---- 填满整个屏幕 ----
        for (y = 0; y < vinfo->yres; y++) {
            uint8_t *row =
                fbmem + y * finfo->line_length;

            for (x = 0; x < vinfo->xres; x++) {
                uint8_t *pixel = row + x * bytes_per_pixel;

                // 假设是 RGB24 或 BGR32
                switch (vinfo->bits_per_pixel) {
                    case 32:    // ARGB 或 BGRA
                        pixel[0] = b;
                        pixel[1] = g;
                        pixel[2] = r;
                        pixel[3] = 0xff;
                        break;

                    case 24:    // BGR
                        pixel[0] = b;
                        pixel[1] = g;
                        pixel[2] = r;
                        break;

                    case 16: { // RGB565
                        uint16_t color =
                            ((r & 0xF8) << 8) |
                            ((g & 0xFC) << 3) |
                            (b >> 3);
                        *(uint16_t*)pixel = color;
                        break;
                    }

                    default:
                        printf("Unsupported bpp: %u\n",
                               vinfo->bits_per_pixel);
                        return;
                }
            }
        }

        usleep(100000);
    }

    printf("Draw finished.\n");
}

int main()
{
    int fd = open("/dev/fb0", O_RDWR);
    if (fd < 0) {
        perror("open /dev/fb0");
        return 1;
    }

    struct fb_var_screeninfo vinfo;
    struct fb_fix_screeninfo finfo;

    if (ioctl(fd, FBIOGET_VSCREENINFO, &vinfo)) {
        perror("FBIOGET_VSCREENINFO");
        close(fd);
        return 1;
    }

    if (ioctl(fd, FBIOGET_FSCREENINFO, &finfo)) {
        perror("FBIOGET_FSCREENINFO");
        close(fd);
        return 1;
    }

    printf("Framebuffer info:\n");
    printf("  Resolution: %ux%u\n", vinfo.xres, vinfo.yres);
    printf("  Bits per pixel: %u\n", vinfo.bits_per_pixel);
    printf("  Line length: %u bytes\n", finfo.line_length);
    printf("  MMIO/SMEM start: 0x%lx\n", (unsigned long)finfo.smem_start);
    printf("  Size: %u bytes\n\n", finfo.smem_len);

    size_t fb_size = finfo.smem_len;

    uint8_t *fbmem = mmap(NULL, fb_size,
                          PROT_READ | PROT_WRITE,
                          MAP_SHARED, fd, 0);

    if (fbmem == MAP_FAILED) {
        perror("mmap");
        close(fd);
        return 1;
    }

    printf("mmap returned: %p\n", fbmem);

    // --- 调用绘图 ---
    modeset_draw(fbmem, &vinfo, &finfo);

    munmap(fbmem, fb_size);
    close(fd);
    return 0;
}
