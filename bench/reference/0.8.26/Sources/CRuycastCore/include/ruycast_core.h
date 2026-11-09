#ifndef RUYCAST_CORE_H
#define RUYCAST_CORE_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* UTF-8 JSON strings are NUL-terminated; returned strings are owned by Rust.
   Every returned string/handle must be freed exactly once by its matching free.
   Calls never unwind across this boundary. Invalid control payloads return an
   owned JSON error response; invalid index inputs return NULL/-1.
   Pointers must be valid for their declared lengths; null cannot encode a slice. */
uint32_t rustcast_abi_version(void);
char *rustcast_call(const char *operation, const char *payload);
void rustcast_free(char *response);
typedef struct RuycastSearchIndex RuycastSearchIndex;
RuycastSearchIndex *rustcast_index_new(const char *catalog);
/* Index is immutable. Query is UTF-8 with explicit byte length, allowing NULs.
   Each call borrows all buffers synchronously and retains none. Output capacity
   is in size_t elements; return is count written or -1 for invalid input/panic.
   Recency is empty or exactly catalog-aligned, finite and nonnegative. */
ptrdiff_t rustcast_index_search_ranked_into(const RuycastSearchIndex *index,
    const uint8_t *query, size_t length, const double *recency, size_t recency_count,
    size_t *output, size_t capacity);
void rustcast_index_free(RuycastSearchIndex *index);
#ifdef __cplusplus
}
#endif
#endif
