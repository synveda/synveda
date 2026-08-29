#define _DARWIN_C_SOURCE 1
#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#define SNAPSHOT_MAX_BYTES 4096U

#if defined(__APPLE__)
#define STAT_MTIME(value) ((value).st_mtimespec)
#define STAT_CTIME(value) ((value).st_ctimespec)
#else
#define STAT_MTIME(value) ((value).st_mtim)
#define STAT_CTIME(value) ((value).st_ctim)
#endif

static int same_timespec(struct timespec left, struct timespec right) {
    return left.tv_sec == right.tv_sec && left.tv_nsec == right.tv_nsec;
}

static int same_source(const struct stat *left, const struct stat *right) {
    return left->st_dev == right->st_dev && left->st_ino == right->st_ino &&
           left->st_size == right->st_size &&
           same_timespec(STAT_MTIME(*left), STAT_MTIME(*right)) &&
           same_timespec(STAT_CTIME(*left), STAT_CTIME(*right));
}

static int write_all(int descriptor, const uint8_t *bytes, size_t length) {
    size_t written = 0;
    while (written < length) {
        ssize_t result = write(descriptor, bytes + written, length - written);
        if (result < 0 && errno == EINTR) {
            continue;
        }
        if (result <= 0) {
            return -1;
        }
        written += (size_t)result;
    }
    return 0;
}

int main(int argc, char **argv) {
    struct stat path_before;
    struct stat opened_before;
    struct stat opened_after;
    struct stat path_after;
    uint8_t bytes[SNAPSHOT_MAX_BYTES + 1U];
    size_t length = 0;
    int source = -1;
    int destination = -1;
    int destination_created = 0;
    int success = 0;

    if (argc != 3) {
        return 64;
    }
    if (lstat(argv[1], &path_before) < 0 || !S_ISREG(path_before.st_mode)) {
        goto done;
    }
    source = open(argv[1], O_RDONLY | O_NONBLOCK | O_NOFOLLOW | O_CLOEXEC);
    if (source < 0 || fstat(source, &opened_before) < 0 ||
        !S_ISREG(opened_before.st_mode) ||
        !same_source(&path_before, &opened_before) || opened_before.st_size < 0 ||
        (uintmax_t)opened_before.st_size > SNAPSHOT_MAX_BYTES) {
        goto done;
    }

    while (length < sizeof(bytes)) {
        ssize_t result = read(source, bytes + length, sizeof(bytes) - length);
        if (result < 0 && errno == EINTR) {
            continue;
        }
        if (result < 0 || result == 0) {
            if (result < 0) {
                goto done;
            }
            break;
        }
        length += (size_t)result;
    }
    if (length > SNAPSHOT_MAX_BYTES || fstat(source, &opened_after) < 0 ||
        lstat(argv[1], &path_after) < 0 || !S_ISREG(path_after.st_mode) ||
        !same_source(&opened_before, &opened_after) ||
        !same_source(&opened_after, &path_after) ||
        (uintmax_t)opened_after.st_size != length) {
        goto done;
    }

    destination = open(
        argv[2],
        O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC,
        S_IRUSR | S_IWUSR
    );
    if (destination < 0) {
        goto done;
    }
    destination_created = 1;
    if (fchmod(destination, S_IRUSR | S_IWUSR) < 0 ||
        write_all(destination, bytes, length) < 0 || fsync(destination) < 0) {
        goto done;
    }
    if (close(destination) < 0) {
        destination = -1;
        goto done;
    }
    destination = -1;
    success = 1;

done:
    if (source >= 0) {
        (void)close(source);
    }
    if (destination >= 0) {
        (void)close(destination);
    }
    if (!success && destination_created) {
        (void)unlink(argv[2]);
    }
    if (!success) {
        return 1;
    }
    return 0;
}
