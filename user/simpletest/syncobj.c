#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <time.h>
#include <unistd.h>

#include <xf86drm.h>
#include <drm/drm.h>
#include <drm/virtgpu_drm.h>

#ifndef VIRTGPU_EXECBUF_RING_IDX
#define VIRTGPU_EXECBUF_RING_IDX 0x04
#endif

#ifndef VIRTGPU_EXECBUF_SYNCOBJ_RESET
#define VIRTGPU_EXECBUF_SYNCOBJ_RESET 0x01
#endif

#ifndef VIRTGPU_PARAM_CONTEXT_INIT
#define VIRTGPU_PARAM_CONTEXT_INIT 6
#endif

/*
 * Some libdrm headers in the local environment still expose the old
 * `drm_virtgpu_execbuffer` without syncobj fields. Define the ABI shape
 * explicitly here so the example always shows the modern submit path:
 *
 *   SYNCOBJ_CREATE
 *   -> VIRTGPU_EXECBUFFER(in_syncobjs/out_syncobjs)
 *   -> SYNCOBJ_WAIT
 *   -> SYNCOBJ_DESTROY
 */
struct virtgpu_execbuffer_syncobj_abi {
    uint32_t handle;
    uint32_t flags;
    uint64_t point;
};

struct virtgpu_execbuffer_abi {
    uint32_t flags;
    uint32_t size;
    uint64_t command;
    uint64_t bo_handles;
    uint32_t num_bo_handles;
    int32_t fence_fd;
    uint32_t ring_idx;
    uint32_t syncobj_stride;
    uint32_t num_in_syncobjs;
    uint32_t num_out_syncobjs;
    uint64_t in_syncobjs;
    uint64_t out_syncobjs;
};

#define DRM_IOCTL_VIRTGPU_EXECBUFFER_ABI \
    DRM_IOWR(DRM_COMMAND_BASE + DRM_VIRTGPU_EXECBUFFER, struct virtgpu_execbuffer_abi)

static uint64_t user_ptr(const void *ptr)
{
    return (uint64_t)(uintptr_t)ptr;
}

static int64_t monotonic_deadline_ns_from_now(int64_t delta_ns)
{
    struct timespec now;

    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) {
        perror("clock_gettime(CLOCK_MONOTONIC)");
        return -1;
    }

    return (int64_t)now.tv_sec * 1000000000LL + now.tv_nsec + delta_ns;
}

static void print_capability(int fd, uint64_t cap, const char *name)
{
    uint64_t value = 0;
    int ret;

    ret = drmGetCap(fd, cap, &value);
    if (ret != 0) {
        printf("drmGetCap(%s) failed: ret=%d errno=%d (%s)\n",
               name, ret, errno, strerror(errno));
        return;
    }

    printf("%s = %" PRIu64 "\n", name, value);
}

static void print_getparam(int fd, uint64_t param, const char *name)
{
    uint64_t value = 0;
    struct drm_virtgpu_getparam getparam = {
        .param = param,
        .value = user_ptr(&value),
    };
    int ret;

    ret = ioctl(fd, DRM_IOCTL_VIRTGPU_GETPARAM, &getparam);
    if (ret != 0) {
        printf("DRM_IOCTL_VIRTGPU_GETPARAM(%s) failed: ret=%d errno=%d (%s)\n",
               name, ret, errno, strerror(errno));
        return;
    }

    printf("%s = %" PRIu64 "\n", name, value);
}

static void print_syncobj_create(const struct drm_syncobj_create *create)
{
    printf("submit DRM_IOCTL_SYNCOBJ_CREATE {\n");
    printf("  handle = %u\n", create->handle);
    printf("  flags  = 0x%x\n", create->flags);
    printf("}\n");
}

static void print_syncobj_destroy(const struct drm_syncobj_destroy *destroy)
{
    printf("submit DRM_IOCTL_SYNCOBJ_DESTROY {\n");
    printf("  handle = %u\n", destroy->handle);
    printf("  pad    = %u\n", destroy->pad);
    printf("}\n");
}

static void print_syncobj_wait(const struct drm_syncobj_wait *wait,
                               const uint32_t *handles)
{
    uint32_t index;

    printf("submit DRM_IOCTL_SYNCOBJ_WAIT {\n");
    printf("  handles        = 0x%" PRIx64 "\n", wait->handles);
    printf("  timeout_nsec   = %" PRId64 "\n", wait->timeout_nsec);
    printf("  count_handles  = %u\n", wait->count_handles);
    printf("  flags          = 0x%x\n", wait->flags);
    printf("  first_signaled = %u\n", wait->first_signaled);
    for (index = 0; index < wait->count_handles; index++) {
        printf("  handles[%u]    = %u\n", index, handles[index]);
    }
    printf("}\n");
}

static void print_execbuffer_syncobjs(const char *name,
                                      const struct virtgpu_execbuffer_syncobj_abi *syncobjs,
                                      uint32_t count)
{
    uint32_t index;

    for (index = 0; index < count; index++) {
        printf("  %s[%u].handle = %u\n", name, index, syncobjs[index].handle);
        printf("  %s[%u].flags  = 0x%x\n", name, index, syncobjs[index].flags);
        printf("  %s[%u].point  = %" PRIu64 "\n", name, index, syncobjs[index].point);
    }
}

static void print_execbuffer_submit(const struct virtgpu_execbuffer_abi *exec,
                                    const uint32_t *bo_handles,
                                    const struct virtgpu_execbuffer_syncobj_abi *in_syncobjs,
                                    const struct virtgpu_execbuffer_syncobj_abi *out_syncobjs)
{
    uint32_t index;
    const uint32_t *command_words = (const uint32_t *)(uintptr_t)exec->command;
    uint32_t command_dwords = exec->size / sizeof(uint32_t);

    printf("submit DRM_IOCTL_VIRTGPU_EXECBUFFER {\n");
    printf("  flags          = 0x%x\n", exec->flags);
    printf("  size           = %u\n", exec->size);
    printf("  command        = 0x%" PRIx64 "\n", exec->command);
    printf("  bo_handles     = 0x%" PRIx64 "\n", exec->bo_handles);
    printf("  num_bo_handles = %u\n", exec->num_bo_handles);
    printf("  fence_fd       = %d\n", exec->fence_fd);
    printf("  ring_idx       = %u\n", exec->ring_idx);
    printf("  syncobj_stride = %u\n", exec->syncobj_stride);
    printf("  num_in_syncobjs  = %u\n", exec->num_in_syncobjs);
    printf("  num_out_syncobjs = %u\n", exec->num_out_syncobjs);
    printf("  in_syncobjs      = 0x%" PRIx64 "\n", exec->in_syncobjs);
    printf("  out_syncobjs     = 0x%" PRIx64 "\n", exec->out_syncobjs);
    for (index = 0; index < exec->num_bo_handles; index++) {
        printf("  bo_handles[%u] = %u\n", index, bo_handles[index]);
    }
    for (index = 0; index < command_dwords; index++) {
        printf("  command[%u]    = 0x%08" PRIx32 "\n", index, command_words[index]);
    }
    print_execbuffer_syncobjs("in_syncobjs", in_syncobjs, exec->num_in_syncobjs);
    print_execbuffer_syncobjs("out_syncobjs", out_syncobjs, exec->num_out_syncobjs);
    printf("}\n");
}

static int do_syncobj_create(int fd, uint32_t flags, uint32_t *handle_out)
{
    struct drm_syncobj_create create = {
        .handle = 0,
        .flags = flags,
    };
    int ret;

    print_syncobj_create(&create);
    ret = ioctl(fd, DRM_IOCTL_SYNCOBJ_CREATE, &create);
    if (ret != 0) {
        printf("DRM_IOCTL_SYNCOBJ_CREATE failed: ret=%d errno=%d (%s)\n",
               ret, errno, strerror(errno));
        return -1;
    }

    printf("receive DRM_IOCTL_SYNCOBJ_CREATE {\n");
    printf("  ret    = %d\n", ret);
    printf("  handle = %u\n", create.handle);
    printf("  flags  = 0x%x\n", create.flags);
    printf("}\n");

    *handle_out = create.handle;
    return 0;
}

static int do_syncobj_wait(int fd,
                           uint32_t *handles,
                           uint32_t count_handles,
                           int64_t timeout_nsec,
                           uint32_t flags,
                           const char *label)
{
    struct drm_syncobj_wait wait = {
        .handles = user_ptr(handles),
        .timeout_nsec = timeout_nsec,
        .count_handles = count_handles,
        .flags = flags,
        .first_signaled = 0,
        .pad = 0,
    };
    int ret;

    printf("\n[%s]\n", label);
    print_syncobj_wait(&wait, handles);
    ret = ioctl(fd, DRM_IOCTL_SYNCOBJ_WAIT, &wait);
    if (ret != 0) {
        printf("DRM_IOCTL_SYNCOBJ_WAIT failed: ret=%d errno=%d (%s)\n",
               ret, errno, strerror(errno));
        return -1;
    }

    printf("receive DRM_IOCTL_SYNCOBJ_WAIT {\n");
    printf("  ret            = %d\n", ret);
    printf("  first_signaled = %u\n", wait.first_signaled);
    printf("}\n");
    return 0;
}

static int do_syncobj_destroy(int fd, uint32_t handle)
{
    struct drm_syncobj_destroy destroy = {
        .handle = handle,
        .pad = 0,
    };
    int ret;

    print_syncobj_destroy(&destroy);
    ret = ioctl(fd, DRM_IOCTL_SYNCOBJ_DESTROY, &destroy);
    if (ret != 0) {
        printf("DRM_IOCTL_SYNCOBJ_DESTROY failed: ret=%d errno=%d (%s)\n",
               ret, errno, strerror(errno));
        return -1;
    }

    printf("receive DRM_IOCTL_SYNCOBJ_DESTROY { ret = %d }\n", ret);
    return 0;
}

static int do_virtgpu_execbuffer_submit(
    int fd,
    const char *label,
    const uint32_t *command_stream,
    uint32_t command_size,
    const uint32_t *bo_handles,
    uint32_t num_bo_handles,
    const struct virtgpu_execbuffer_syncobj_abi *in_syncobjs,
    uint32_t num_in_syncobjs,
    const struct virtgpu_execbuffer_syncobj_abi *out_syncobjs,
    uint32_t num_out_syncobjs)
{
    struct virtgpu_execbuffer_abi exec = {
        .flags = 0,
        .size = command_size,
        .command = user_ptr(command_stream),
        .bo_handles = user_ptr(bo_handles),
        .num_bo_handles = num_bo_handles,
        .fence_fd = -1,
        .ring_idx = 0,
        .syncobj_stride = sizeof(*out_syncobjs),
        .num_in_syncobjs = num_in_syncobjs,
        .num_out_syncobjs = num_out_syncobjs,
        .in_syncobjs = user_ptr(in_syncobjs),
        .out_syncobjs = user_ptr(out_syncobjs),
    };
    int ret;

    printf("\n[%s]\n", label);
    print_execbuffer_submit(&exec, bo_handles, in_syncobjs, out_syncobjs);
    ret = ioctl(fd, DRM_IOCTL_VIRTGPU_EXECBUFFER_ABI, &exec);
    if (ret != 0) {
        printf("DRM_IOCTL_VIRTGPU_EXECBUFFER failed: ret=%d errno=%d (%s)\n",
               ret, errno, strerror(errno));
        return -1;
    }

    printf("receive DRM_IOCTL_VIRTGPU_EXECBUFFER {\n");
    printf("  ret      = %d\n", ret);
    printf("  fence_fd = %d\n", exec.fence_fd);
    printf("}\n");
    return 0;
}

int main(int argc, char **argv)
{
    const char *card = "/dev/dri/card0";
    int fd;
    uint32_t render_done_handle = 0;
    uint32_t second_stage_done_handle = 0;
    uint32_t wait_handles[1];

    /*
     * Placeholder commands only describe the userspace ABI shape. A real virtio-gpu
     * renderer will validate the command payload according to the selected context.
     */
    uint32_t first_command_stream[] = {
        0x00000000,
        0x00000000,
        0x00000000,
        0x00000000,
    };
    uint32_t second_command_stream[] = {
        0x11111111,
        0x22222222,
        0x33333333,
        0x44444444,
    };
    struct virtgpu_execbuffer_syncobj_abi first_submit_out[1];
    struct virtgpu_execbuffer_syncobj_abi second_submit_in[1];
    struct virtgpu_execbuffer_syncobj_abi second_submit_out[1];
    int64_t deadline;

    if (argc > 1) {
        card = argv[1];
    }

    fd = open(card, O_RDWR | O_CLOEXEC);
    if (fd < 0) {
        perror("open drm device");
        return 1;
    }

    printf("opened %s\n", card);
    print_capability(fd, DRM_CAP_SYNCOBJ, "DRM_CAP_SYNCOBJ");
    print_capability(fd, DRM_CAP_SYNCOBJ_TIMELINE, "DRM_CAP_SYNCOBJ_TIMELINE");
    print_getparam(fd, VIRTGPU_PARAM_3D_FEATURES, "VIRTGPU_PARAM_3D_FEATURES");
    print_getparam(fd, VIRTGPU_PARAM_CONTEXT_INIT, "VIRTGPU_PARAM_CONTEXT_INIT");
    printf("\n");

    /*
     * Stage 1:
     *   userspace 创建一个 syncobj，后续 submit 会把“完成 fence”写进这个容器。
     */
    if (do_syncobj_create(fd, 0, &render_done_handle) != 0) {
        close(fd);
        return 1;
    }

    first_submit_out[0].handle = render_done_handle;
    first_submit_out[0].flags = 0;
    first_submit_out[0].point = 0;

    /*
     * Stage 2:
     *   userspace 发出第一次 virtgpu execbuffer submit。
     *   这个 submit 没有 in_syncobjs，但有一个 out_syncobj。
     *
     *   内核需要做的事情通常包括：
     *   1. 解析 execbuffer ABI；
     *   2. 拷贝 out_syncobjs 数组；
     *   3. 为这次 submission 创建 completion fence；
     *   4. 把 fence 安装到 render_done_handle 对应的 syncobj 里。
     */
    do_virtgpu_execbuffer_submit(fd,
                                 "virtgpu submit #1: signal render_done_handle",
                                 first_command_stream,
                                 sizeof(first_command_stream),
                                 NULL,
                                 0,
                                 NULL,
                                 0,
                                 first_submit_out,
                                 1);

    wait_handles[0] = render_done_handle;
    deadline = monotonic_deadline_ns_from_now(1000 * 1000);
    if (deadline >= 0) {
        do_syncobj_wait(fd,
                        wait_handles,
                        1,
                        deadline,
                        0,
                        "wait for render_done_handle after submit #1");
    }

    /*
     * Stage 3:
     *   第二次 submit 显式依赖第一次 submit 产生的 syncobj。
     *   userspace 把 render_done_handle 放进 in_syncobjs，同时再指定一个新的 out_syncobj。
     */
    if (do_syncobj_create(fd, 0, &second_stage_done_handle) != 0) {
        do_syncobj_destroy(fd, render_done_handle);
        close(fd);
        return 1;
    }

    second_submit_in[0].handle = render_done_handle;
    second_submit_in[0].flags = 0;
    second_submit_in[0].point = 0;

    second_submit_out[0].handle = second_stage_done_handle;
    second_submit_out[0].flags = VIRTGPU_EXECBUF_SYNCOBJ_RESET;
    second_submit_out[0].point = 0;

    /*
     * 这里展示了更完整的依赖链：
     *   submit #2 等待 render_done_handle
     *   submit #2 完成后 signal second_stage_done_handle
     */
    do_virtgpu_execbuffer_submit(fd,
                                 "virtgpu submit #2: wait render_done_handle, signal second_stage_done_handle",
                                 second_command_stream,
                                 sizeof(second_command_stream),
                                 NULL,
                                 0,
                                 second_submit_in,
                                 1,
                                 second_submit_out,
                                 1);

    wait_handles[0] = second_stage_done_handle;
    deadline = monotonic_deadline_ns_from_now(1000 * 1000);
    if (deadline >= 0) {
        do_syncobj_wait(fd,
                        wait_handles,
                        1,
                        deadline,
                        0,
                        "wait for second_stage_done_handle after submit #2");
    }

    do_syncobj_destroy(fd, second_stage_done_handle);
    do_syncobj_destroy(fd, render_done_handle);

    close(fd);
    return 0;
}
