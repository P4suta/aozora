#ifndef TREE_SITTER_ARRAY_H_
#define TREE_SITTER_ARRAY_H_

#ifdef __cplusplus
extern "C" {
#endif

#include "./alloc.h"

#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#ifdef _MSC_VER
#pragma warning(push)
#pragma warning(disable : 4101)
#elif defined(__GNUC__) || defined(__clang__)
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-variable"
#endif

#define Array(T)       \
  struct {             \
    T *contents;       \
    uint32_t size;     \
    uint32_t capacity; \
  }

/// Initialize an array.
#define array_init(self) \
  ((self)->size = 0, (self)->capacity = 0, (self)->contents = NULL)

/// Create an empty array.
#define array_new() \
  { NULL, 0, 0 }

/// Get a pointer to the element at a given `index` in the array.
#define array_get(self, _index) \
  (assert((uint32_t)(_index) < (self)->size), &(self)->contents[_index])

/// Get a pointer to the first element in the array.
#define array_front(self) array_get(self, 0)

/// Get a pointer to the last element in the array.
#define array_back(self) array_get(self, (self)->size - 1)

/// Sets the array size to zero without releasing its storage.
#define array_clear(self) ((self)->size = 0)

/// Reserves space for at least `new_capacity` elements without shrinking existing storage.
#define array_reserve(self, new_capacity)        \
  ((self)->contents = _array__reserve(           \
    (void *)(self)->contents, &(self)->capacity, \
    array_elem_size(self), new_capacity)         \
  )

/// Releases the array's storage without freeing allocations owned by its elements.
#define array_delete(self)                           \
  do {                                               \
    if ((self)->contents) ts_free((self)->contents); \
    (self)->contents = NULL;                         \
    (self)->size = 0;                                \
    (self)->capacity = 0;                            \
  } while (0)

/// Push a new `element` onto the end of the array.
#define array_push(self, element)                                 \
  do {                                                            \
    (self)->contents = _array__grow(                              \
      (void *)(self)->contents, (self)->size, &(self)->capacity,  \
      1, array_elem_size(self)                                    \
    );                                                            \
   (self)->contents[(self)->size++] = (element);                  \
  } while(0)

/// Increase the array's size by `count` elements.
/// Initializes new elements to zero.
#define array_grow_by(self, count)                                               \
  do {                                                                           \
    if ((count) == 0) break;                                                     \
    (self)->contents = _array__grow(                                             \
      (self)->contents, (self)->size, &(self)->capacity,                         \
      count, array_elem_size(self)                                               \
    );                                                                           \
    memset((self)->contents + (self)->size, 0, (count) * array_elem_size(self)); \
    (self)->size += (count);                                                     \
  } while (0)

/// Append all elements from one array to the end of another.
#define array_push_all(self, other) \
  array_extend((self), (other)->size, (other)->contents)

/// Append `count` elements to the end of the array, reading their values from the `contents` pointer.
#define array_extend(self, count, other_contents)                 \
  (self)->contents = _array__splice(                              \
    (void*)(self)->contents, &(self)->size, &(self)->capacity,    \
    array_elem_size(self), (self)->size, 0, count, other_contents \
  )

/// Remove `old_count` elements from the array starting at the given `index`.
/// At the same index, insert `new_count` new elements, reading their values from the `new_contents` pointer.
#define array_splice(self, _index, old_count, new_count, new_contents) \
  (self)->contents = _array__splice(                                   \
    (void *)(self)->contents, &(self)->size, &(self)->capacity,        \
    array_elem_size(self), _index, old_count, new_count, new_contents  \
  )

/// Insert one `element` into the array at the given `index`.
#define array_insert(self, _index, element)                     \
  (self)->contents = _array__splice(                            \
    (void *)(self)->contents, &(self)->size, &(self)->capacity, \
    array_elem_size(self), _index, 0, 1, &(element)             \
  )

/// Remove one element from the array at the given `index`.
#define array_erase(self, _index) \
  _array__erase((void *)(self)->contents, &(self)->size, array_elem_size(self), _index)

/// Pop the last element off the array, returning the element by value.
#define array_pop(self) ((self)->contents[--(self)->size])

/// Copies another array's contents, growing storage when needed.
#define array_assign(self, other)                                   \
  (self)->contents = _array__assign(                                \
    (void *)(self)->contents, &(self)->size, &(self)->capacity,     \
    (const void *)(other)->contents, (other)->size, array_elem_size(self) \
  )

/// Swaps the contents and capacities of two arrays.
#define array_swap(self, other)                                     \
  do {                                                              \
    void *_array_swap_tmp = (void *)(self)->contents;               \
    (self)->contents = (other)->contents;                           \
    (other)->contents = _array_swap_tmp;                            \
    _array__swap(&(self)->size, &(self)->capacity,                  \
                 &(other)->size, &(other)->capacity);               \
  } while (0)

/// Returns the size of one array element in bytes.
#define array_elem_size(self) (sizeof *(self)->contents)

/// Searches a sorted array using `compare`, writing a matching index and `true` to `exists` on a match.
/// Otherwise writes the insertion index and `false` to `exists`.
#define array_search_sorted_with(self, compare, needle, _index, _exists) \
  _array__search_sorted(self, 0, compare, , needle, _index, _exists)

/// Searches a sorted array by an integer field written with a leading dot.
/// Writes the matching or insertion index and whether a match exists.
#define array_search_sorted_by(self, field, needle, _index, _exists) \
  _array__search_sorted(self, 0, _compare_int, field, needle, _index, _exists)

/// Inserts `value` into a sorted array using `compare` unless an equal value exists.
#define array_insert_sorted_with(self, compare, value) \
  do { \
    unsigned _index, _exists; \
    array_search_sorted_with(self, compare, &(value), &_index, &_exists); \
    if (!_exists) array_insert(self, _index, value); \
  } while (0)

/// Inserts `value` into a sorted array by an integer field unless an equal value exists.
#define array_insert_sorted_by(self, field, value) \
  do { \
    unsigned _index, _exists; \
    array_search_sorted_by(self, field, (value) field, &_index, &_exists); \
    if (!_exists) array_insert(self, _index, value); \
  } while (0)

// SAFETY: field pointers avoid aliasing whole array structs with different element types.

static inline void _array__erase(void* self_contents, uint32_t *size,
                                size_t element_size, uint32_t index) {
  assert(index < *size);
  char *contents = (char *)self_contents;
  memmove(contents + index * element_size, contents + (index + 1) * element_size,
          (*size - index - 1) * element_size);
  (*size)--;
}

static inline void *_array__reserve(void *contents, uint32_t *capacity,
                                  size_t element_size, uint32_t new_capacity) {
  void *new_contents = contents;
  if (new_capacity > *capacity) {
    if (contents) {
      new_contents = ts_realloc(contents, new_capacity * element_size);
    } else {
      new_contents = ts_malloc(new_capacity * element_size);
    }
    *capacity = new_capacity;
  }
  return new_contents;
}

static inline void *_array__assign(void* self_contents, uint32_t *self_size, uint32_t *self_capacity,
                                 const void *other_contents, uint32_t other_size, size_t element_size) {
  void *new_contents = _array__reserve(self_contents, self_capacity, element_size, other_size);
  *self_size = other_size;
  memcpy(new_contents, other_contents, *self_size * element_size);
  return new_contents;
}

static inline void _array__swap(uint32_t *self_size, uint32_t *self_capacity,
                               uint32_t *other_size, uint32_t *other_capacity) {
  uint32_t tmp_size = *self_size;
  uint32_t tmp_capacity = *self_capacity;
  *self_size = *other_size;
  *self_capacity = *other_capacity;
  *other_size = tmp_size;
  *other_capacity = tmp_capacity;
}

static inline void *_array__grow(void *contents, uint32_t size, uint32_t *capacity,
                               uint32_t count, size_t element_size) {
  void *new_contents = contents;
  uint32_t new_size = size + count;
  if (new_size > *capacity) {
    uint32_t new_capacity = *capacity * 2;
    if (new_capacity < 8) new_capacity = 8;
    if (new_capacity < new_size) new_capacity = new_size;
    new_contents = _array__reserve(contents, capacity, element_size, new_capacity);
  }
  return new_contents;
}

static inline void *_array__splice(void *self_contents, uint32_t *size, uint32_t *capacity,
                                 size_t element_size,
                                 uint32_t index, uint32_t old_count,
                                 uint32_t new_count, const void *elements) {
  uint32_t new_size = *size + new_count - old_count;
  uint32_t old_end = index + old_count;
  uint32_t new_end = index + new_count;
  assert(old_end <= *size);

  void *new_contents = _array__reserve(self_contents, capacity, element_size, new_size);

  char *contents = (char *)new_contents;
  if (*size > old_end) {
    memmove(
      contents + new_end * element_size,
      contents + old_end * element_size,
      (*size - old_end) * element_size
    );
  }
  if (new_count > 0) {
    if (elements) {
      memcpy(
        (contents + index * element_size),
        elements,
        new_count * element_size
      );
    } else {
      memset(
        (contents + index * element_size),
        0,
        new_count * element_size
      );
    }
  }
  *size += new_count - old_count;

  return new_contents;
}

#define _array__search_sorted(self, start, compare, suffix, needle, _index, _exists) \
  do { \
    *(_index) = start; \
    *(_exists) = false; \
    uint32_t size = (self)->size - *(_index); \
    if (size == 0) break; \
    int comparison; \
    while (size > 1) { \
      uint32_t half_size = size / 2; \
      uint32_t mid_index = *(_index) + half_size; \
      comparison = compare(&((self)->contents[mid_index] suffix), (needle)); \
      if (comparison <= 0) *(_index) = mid_index; \
      size -= half_size; \
    } \
    comparison = compare(&((self)->contents[*(_index)] suffix), (needle)); \
    if (comparison == 0) *(_exists) = true; \
    else if (comparison < 0) *(_index) += 1; \
  } while (0)

#define _compare_int(a, b) ((int)*(a) - (int)(b))

#ifdef _MSC_VER
#pragma warning(pop)
#elif defined(__GNUC__) || defined(__clang__)
#pragma GCC diagnostic pop
#endif

#ifdef __cplusplus
}
#endif

#endif
