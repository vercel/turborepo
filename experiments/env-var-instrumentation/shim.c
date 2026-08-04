/* LD_PRELOAD interposer that logs every getenv()/secure_getenv() call.
 *
 * Log destination: the file named by ENV_AUDIT_OUT (looked up via the real
 * getenv, guarded against recursion). Each line: "<pid>\t<progname>\t<var>".
 * Uses raw open/write (no stdio) to stay async-signal-safe-ish and avoid
 * re-entrancy through malloc/stdio.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <string.h>
#include <unistd.h>
#include <stdlib.h>
#include <errno.h>

extern char *program_invocation_short_name;

typedef char *(*getenv_fn)(const char *);

static getenv_fn real_getenv;
static __thread int in_hook; /* recursion guard */

static void log_access(const char *name) {
    static const char *outpath;
    if (!outpath) {
        if (!real_getenv)
            real_getenv = (getenv_fn)dlsym(RTLD_NEXT, "getenv");
        if (real_getenv)
            outpath = real_getenv("ENV_AUDIT_OUT");
    }
    if (!outpath) return;

    int fd = open(outpath, O_WRONLY | O_CREAT | O_APPEND, 0644);
    if (fd < 0) return;

    char buf[512];
    size_t n = 0;
    pid_t pid = getpid();
    /* write pid as decimal */
    char pidbuf[16];
    int pi = 0;
    if (pid == 0) pidbuf[pi++] = '0';
    while (pid > 0 && pi < 15) { pidbuf[pi++] = '0' + (pid % 10); pid /= 10; }
    while (pi > 0 && n < sizeof(buf) - 1) buf[n++] = pidbuf[--pi];
    buf[n++] = '\t';
    const char *prog = program_invocation_short_name ? program_invocation_short_name : "?";
    while (*prog && n < sizeof(buf) - 2) buf[n++] = *prog++;
    buf[n++] = '\t';
    while (*name && n < sizeof(buf) - 2) buf[n++] = *name++;
    buf[n++] = '\n';

    ssize_t rc = write(fd, buf, n);
    (void)rc;
    close(fd);
}

char *getenv(const char *name) {
    if (!real_getenv)
        real_getenv = (getenv_fn)dlsym(RTLD_NEXT, "getenv");
    if (!real_getenv) return NULL;
    if (!in_hook && name) {
        in_hook = 1;
        log_access(name);
        in_hook = 0;
    }
    return real_getenv(name);
}

char *secure_getenv(const char *name) {
    static getenv_fn real_secure;
    if (!real_secure)
        real_secure = (getenv_fn)dlsym(RTLD_NEXT, "secure_getenv");
    if (!real_secure) return NULL;
    if (!in_hook && name) {
        in_hook = 1;
        log_access(name);
        in_hook = 0;
    }
    return real_secure(name);
}
