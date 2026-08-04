/* LD_PRELOAD process-tree census: every dynamically-linked process
 * self-announces its executable at load, and exec* / posix_spawn* calls
 * log the binary they are about to run. Lines:
 *   "<pid>\tself\t<exe>"  or  "<pid>\texec\t<target>"
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <string.h>
#include <unistd.h>
#include <spawn.h>
#include <stdlib.h>

static void note(const char *tag, const char *path) {
    const char *out = getenv("EXEC_AUDIT_OUT");
    if (!out) return;
    int fd = open(out, O_WRONLY | O_CREAT | O_APPEND, 0644);
    if (fd < 0) return;
    char buf[600]; size_t n = 0;
    pid_t pid = getpid(); char pb[16]; int pi = 0;
    if (!pid) pb[pi++] = '0';
    while (pid > 0 && pi < 15) { pb[pi++] = '0' + pid % 10; pid /= 10; }
    while (pi) buf[n++] = pb[--pi];
    buf[n++] = '\t';
    while (*tag && n < sizeof buf - 2) buf[n++] = *tag++;
    buf[n++] = '\t';
    while (*path && n < sizeof buf - 2) buf[n++] = *path++;
    buf[n++] = '\n';
    ssize_t r = write(fd, buf, n); (void)r;
    close(fd);
}

__attribute__((constructor)) static void announce(void) {
    char exe[512]; ssize_t k = readlink("/proc/self/exe", exe, sizeof exe - 1);
    if (k > 0) { exe[k] = 0; note("self", exe); }
}

int execve(const char *path, char *const argv[], char *const envp[]) {
    static int (*real)(const char *, char *const[], char *const[]);
    if (!real) real = dlsym(RTLD_NEXT, "execve");
    note("exec", path);
    return real(path, argv, envp);
}
int execvp(const char *file, char *const argv[]) {
    static int (*real)(const char *, char *const[]);
    if (!real) real = dlsym(RTLD_NEXT, "execvp");
    note("exec", file);
    return real(file, argv);
}
int posix_spawn(pid_t *pid, const char *path,
                const posix_spawn_file_actions_t *fa,
                const posix_spawnattr_t *attr,
                char *const argv[], char *const envp[]) {
    static int (*real)(pid_t *, const char *, const posix_spawn_file_actions_t *,
                       const posix_spawnattr_t *, char *const[], char *const[]);
    if (!real) real = dlsym(RTLD_NEXT, "posix_spawn");
    note("exec", path);
    return real(pid, path, fa, attr, argv, envp);
}
int posix_spawnp(pid_t *pid, const char *file,
                 const posix_spawn_file_actions_t *fa,
                 const posix_spawnattr_t *attr,
                 char *const argv[], char *const envp[]) {
    static int (*real)(pid_t *, const char *, const posix_spawn_file_actions_t *,
                       const posix_spawnattr_t *, char *const[], char *const[]);
    if (!real) real = dlsym(RTLD_NEXT, "posix_spawnp");
    note("exec", file);
    return real(pid, file, fa, attr, argv, envp);
}
