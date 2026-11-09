#include "ruycast_core.h"
/* Rust's usize/isize ABI matches the platform C size_t/ptrdiff_t widths. */
_Static_assert(sizeof(size_t) == sizeof(uintptr_t), "Rust usize width");
_Static_assert(sizeof(ptrdiff_t) == sizeof(intptr_t), "Rust isize width");
