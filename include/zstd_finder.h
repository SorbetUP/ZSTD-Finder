#ifndef ZSTD_FINDER_H
#define ZSTD_FINDER_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ZstfArchiveHandle ZstfArchiveHandle;

enum {
    ZSTF_OK = 0,
    ZSTF_ERR_INVALID_ARGUMENT = -1,
    ZSTF_ERR_BUFFER_TOO_SMALL = -2,
    ZSTF_ERR_ARCHIVE = -3,
    ZSTF_ERR_PANIC = -127,
};

enum {
    ZSTF_KIND_FILE = 1,
    ZSTF_KIND_DIRECTORY = 2,
    ZSTF_KIND_SYMLINK = 3,
};

typedef struct ZstfEntryMetadata {
    uint32_t kind;
    uint32_t unix_mode;
    uint64_t size;
    int64_t modified_seconds;
    uint32_t modified_nanos;
    uint32_t has_modified;
} ZstfEntryMetadata;

uint16_t zstf_format_version(void);
int32_t zstf_archive_open(const char *path, ZstfArchiveHandle **out_handle);
void zstf_archive_close(ZstfArchiveHandle *handle);
int32_t zstf_archive_entry_count(const ZstfArchiveHandle *handle, size_t *out_count);
int32_t zstf_archive_entry_path(const ZstfArchiveHandle *handle,
                                size_t index,
                                uint8_t *buffer,
                                size_t capacity,
                                size_t *out_len);
int32_t zstf_archive_entry_metadata(const ZstfArchiveHandle *handle,
                                    size_t index,
                                    ZstfEntryMetadata *out_metadata);
int32_t zstf_archive_entry_symlink_target(const ZstfArchiveHandle *handle,
                                          size_t index,
                                          uint8_t *buffer,
                                          size_t capacity,
                                          size_t *out_len);
int32_t zstf_archive_read(const ZstfArchiveHandle *handle,
                          const char *path,
                          uint64_t offset,
                          uint8_t *buffer,
                          size_t length,
                          size_t *out_read);
size_t zstf_last_error(char *buffer, size_t capacity);

#ifdef __cplusplus
}
#endif

#endif
