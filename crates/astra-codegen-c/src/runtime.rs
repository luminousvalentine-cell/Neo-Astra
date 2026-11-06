pub const RUNTIME_HEADER: &str = r#"#ifndef NS_RUNTIME_HEADER
#define NS_RUNTIME_HEADER
#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

typedef struct NS_Gc_Shape NS_Gc_Shape;

#define ASTRA_NORETURN __attribute__((noreturn))

typedef struct { const char* data; size_t len; } NS_Str;
typedef struct { const uint8_t* data; size_t len; } NS_Bytes;
typedef struct { void* data; size_t len; size_t cap; } NS_Array;
typedef struct { void* keys; void* values; size_t len; size_t cap; size_t key_size; size_t val_size; bool key_is_str; const NS_Gc_Shape* key_shape; const NS_Gc_Shape* val_shape; } NS_Map;
typedef struct { char _unit; } NS_Unit;

typedef struct { NS_Str _0; } NS_SliceError;

typedef struct { NS_Str _0; } NS_ParseIntError;
typedef struct { NS_Str _0; } NS_OverflowError;

typedef struct { char _unit; } NS_ChannelError;

typedef struct { NS_Str _0; } NS_TaskError;
typedef struct {
    void* data;
    size_t len;
    size_t cap;
    size_t elem_size;
    const NS_Gc_Shape* elem_shape;
    bool closed;
    void* sync;
} NS_Channel;

typedef struct { uint8_t* base; size_t cap; size_t offset; } NS_Region;

typedef enum {
    NS_SHAPE_KIND_LEAF = 0,
    NS_SHAPE_KIND_ARRAY = 1,
    NS_SHAPE_KIND_RECORD = 2,
    NS_SHAPE_KIND_VARIANT = 3
} NS_Gc_Shape_Kind;
typedef struct { size_t offset; const NS_Gc_Shape* shape; } NS_Gc_Shape_Field;
struct NS_Gc_Shape {
    NS_Gc_Shape_Kind kind;
    size_t elem_size;
    const NS_Gc_Shape* elem;
    const NS_Gc_Shape_Field* fields;
    size_t field_count;
    size_t tag_offset;
    const NS_Gc_Shape* const* variants;
    size_t variant_count;
};
typedef struct { uint32_t marks; size_t bytes; const NS_Gc_Shape* shape; } NS_Gc_Head;

extern const NS_Gc_Shape NS_SHAPE_LEAF;
extern const NS_Gc_Shape NS_SHAPE_STR;
extern const NS_Gc_Shape NS_SHAPE_ARRAY_OF_STR;

extern void astra_print_i8(int8_t x);
extern void astra_print_i16(int16_t x);
extern void astra_print_i32(int32_t x);
extern void astra_print_i64(int64_t x);
extern void astra_print_u8(uint8_t x);
extern void astra_print_u16(uint16_t x);
extern void astra_print_u32(uint32_t x);
extern void astra_print_u64(uint64_t x);
extern void astra_print_usize(size_t x);
extern void astra_print_isize(intptr_t x);
extern void astra_print_f32(float x);
extern void astra_print_f64(double x);
extern void astra_print_bool(bool x);
extern void astra_print_str(NS_Str x);
extern void astra_print_bytes(NS_Bytes x);
extern void astra_print_unit(void);

extern void astra_println_i8(int8_t x);
extern void astra_println_i16(int16_t x);
extern void astra_println_i32(int32_t x);
extern void astra_println_i64(int64_t x);
extern void astra_println_u8(uint8_t x);
extern void astra_println_u16(uint16_t x);
extern void astra_println_u32(uint32_t x);
extern void astra_println_u64(uint64_t x);
extern void astra_println_usize(size_t x);
extern void astra_println_isize(intptr_t x);
extern void astra_println_f32(float x);
extern void astra_println_f64(double x);
extern void astra_println_bool(bool x);
extern void astra_println_str(NS_Str x);
extern void astra_println_bytes(NS_Bytes x);
extern void astra_println_unit(void);

extern void astra_assert(bool cond);
ASTRA_NORETURN extern void astra_panic(const char* msg);
ASTRA_NORETURN extern void astra_abort(const char* msg);

extern void* astra_gc_alloc(const NS_Gc_Shape* shape, size_t n);

extern void astra_gc_free(void* p);

extern void* astra_gc_realloc(void* p, const NS_Gc_Shape* shape, size_t n);

extern void* astra_manual_alloc(size_t n);
extern void astra_manual_free(void* p);

extern size_t astra_manual_live_count(void);
extern size_t astra_manual_total_count(void);

extern NS_Region* astra_region_new(size_t bytes);
extern void* astra_region_alloc(NS_Region* r, size_t n);
extern void astra_region_free(NS_Region* r);

extern void astra_runtime_shutdown(void);

extern size_t astra_gc_frame_enter(size_t slots);
extern void astra_gc_frame_exit(size_t rf);
extern void astra_gc_publish(size_t index, void* payload);
extern size_t astra_gc_temp_enter(void);
extern void astra_gc_temp_exit(size_t index);
extern void astra_gc_maybe_collect(void);

extern void astra_gc_collect(void);
extern size_t astra_gc_collected_count(void);
extern size_t astra_gc_root_count(void);

extern void astra_gc_pin(void* payload);
extern size_t astra_gc_pin_count(void);

extern size_t astra_gc_scope_enter(void);
extern void astra_gc_root(size_t scope, const void* payload);
extern void astra_gc_scope_exit(size_t scope);

extern void astra_gc_poison_payload(void* payload);
extern size_t astra_gc_poison_count(void);
extern NS_Array astra_array_new_lit(size_t n, size_t elem_size, const void* src, const NS_Gc_Shape* shape);
extern void astra_array_push(NS_Array* arr, size_t elem_size, const void* value, const NS_Gc_Shape* shape);
extern void* astra_array_index(NS_Array* arr, size_t i, size_t elem_size);

extern size_t astra_str_len(NS_Str s);
extern bool astra_str_is_empty(NS_Str s);
extern bool astra_str_eq(NS_Str a, NS_Str b);
extern int astra_str_cmp(NS_Str a, NS_Str b);

extern bool astra_str_slice_try(NS_Str s, size_t start, size_t end, NS_Str* out);

extern bool astra_str_to_int_try(NS_Str s, int32_t* out);

extern size_t astra_str_char_count(NS_Str s);
extern bool astra_str_char_at(NS_Str s, size_t index, NS_Str* out);
extern NS_Array astra_str_bytes(NS_Str s);
extern NS_Array astra_str_chars(NS_Str s);

extern bool astra_bytes_eq(NS_Bytes a, NS_Bytes b);
extern int astra_bytes_cmp(NS_Bytes a, NS_Bytes b);
extern uint8_t astra_bytes_get(NS_Bytes b, size_t i);

extern void astra_bytes_set(NS_Bytes* b, size_t i, uint8_t value);

extern NS_Bytes astra_bytes_new(size_t len);

extern NS_Bytes astra_bytes_slice(NS_Bytes b, size_t start, size_t end);

extern NS_Map astra_map_new(size_t key_size, size_t val_size, bool key_is_str, const NS_Gc_Shape* key_shape, const NS_Gc_Shape* val_shape);
extern void astra_map_set(NS_Map* m, const void* key, const void* val);
extern void* astra_map_get(const NS_Map* m, const void* key);
extern bool astra_map_contains(const NS_Map* m, const void* key);
extern size_t astra_map_len(const NS_Map* m);
extern void* astra_map_key_at(const NS_Map* m, size_t i);

extern NS_Channel* astra_channel_new(size_t elem_size, const NS_Gc_Shape* elem_shape, const NS_Gc_Shape* handle_shape, const NS_Gc_Shape* data_shape);
extern bool astra_channel_send(NS_Channel* c, const void* value);
extern bool astra_channel_receive(NS_Channel* c, void* out);
extern void astra_channel_close(NS_Channel* c);
extern size_t astra_channel_heap_live_count(void);

typedef struct NS_Heap NS_Heap;
typedef struct {
    uintptr_t handle;
    NS_Heap* heap;
    void (*entry)(void*);
    void* closure;
    size_t value_offset;
    size_t child_count;
    bool started;
    bool finished;
    bool has_value;
    bool panicked;
    char* panic_message;
    volatile bool canceled;
} NS_Task_Core;

extern NS_Heap* astra_heap_create(void);
extern void astra_heap_bind(NS_Heap* heap);
extern void astra_heap_release(NS_Heap* heap);

extern void* astra_gc_alloc_in_heap(NS_Heap* heap, const NS_Gc_Shape* shape, size_t n);
extern void astra_gc_root_in(NS_Heap* heap, void* payload);

extern NS_Heap* astra_current_heap(void);

extern void astra_task_start(NS_Task_Core* task);

extern bool astra_task_join(NS_Task_Core* task);

ASTRA_NORETURN extern void astra_task_raise(const NS_Task_Core* task);
extern void astra_task_heap_free(NS_Task_Core* task);

extern void astra_task_cancel(NS_Task_Core* task);
extern bool astra_task_canceled(void);

extern void astra_task_check(void);

extern void astra_gc_copy_into(NS_Heap* dst, NS_Heap* src, const NS_Gc_Shape* shape, size_t n, void* dst_value, const void* src_value);

extern int8_t  astra_add_i8(int8_t a, int8_t b);
extern int16_t astra_add_i16(int16_t a, int16_t b);
extern int32_t astra_add_i32(int32_t a, int32_t b);
extern int64_t astra_add_i64(int64_t a, int64_t b);
extern int8_t  astra_sub_i8(int8_t a, int8_t b);
extern int16_t astra_sub_i16(int16_t a, int16_t b);
extern int32_t astra_sub_i32(int32_t a, int32_t b);
extern int64_t astra_sub_i64(int64_t a, int64_t b);
extern int8_t  astra_mul_i8(int8_t a, int8_t b);
extern int16_t astra_mul_i16(int16_t a, int16_t b);
extern int32_t astra_mul_i32(int32_t a, int32_t b);
extern int64_t astra_mul_i64(int64_t a, int64_t b);
#endif
"#;

pub const HOST_SHIM: &str = r#"

#include <errno.h>
#include <inttypes.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#if defined(_WIN32)
#include <windows.h>
#else
#include <pthread.h>
#endif

static _Thread_local NS_Task_Core* astra_task_current = NULL;
ASTRA_NORETURN static void astra_task_abort(void);

static void astra_write_str(NS_Str x) {
    if (x.len > 0) {
        fwrite(x.data, 1, x.len, stdout);
    }
}

static void astra_write_f64(double x) {
    if (x != x) { fputs("NaN", stdout); return; }
    if (x == (double)INFINITY) { fputs("Infinity", stdout); return; }
    if (x == (double)-INFINITY) { fputs("-Infinity", stdout); return; }
    double f = x - (double)(int64_t)x;
    if (f == 0.0 && x < 1e16 && x > -1e16) {
        printf("%.1f", x);
    } else {
        printf("%g", x);
    }
}

static void astra_write_f32(float x) { astra_write_f64((double)x); }

#define ASTRA_PRINT_INT(name, type, signed_fmt, unsigned_fmt) \
    void name(type x) { printf(x < 0 ? signed_fmt : unsigned_fmt, x); }

void astra_print_i8(int8_t x) { printf(x < 0 ? "%" PRId8 : "%" PRIu8, x); }
void astra_print_i16(int16_t x) { printf(x < 0 ? "%" PRId16 : "%" PRIu16, x); }
void astra_print_i32(int32_t x) { printf(x < 0 ? "%" PRId32 : "%" PRIu32, x); }
void astra_print_i64(int64_t x) { printf(x < 0 ? "%" PRId64 : "%" PRIu64, x); }
void astra_print_u8(uint8_t x) { printf("%" PRIu8, x); }
void astra_print_u16(uint16_t x) { printf("%" PRIu16, x); }
void astra_print_u32(uint32_t x) { printf("%" PRIu32, x); }
void astra_print_u64(uint64_t x) { printf("%" PRIu64, x); }
void astra_print_usize(size_t x) { printf("%zu", x); }
void astra_print_isize(intptr_t x) { printf("%td", x); }
void astra_print_f32(float x) { astra_write_f32(x); }
void astra_print_f64(double x) { astra_write_f64(x); }
void astra_print_bool(bool x) { fputs(x ? "true" : "false", stdout); }
void astra_print_str(NS_Str x) { astra_write_str(x); }
void astra_print_bytes(NS_Bytes x) {
    for (size_t i = 0; i < x.len; i++) { printf("%02x", x.data[i]); }
}
void astra_print_unit(void) { fputs("()", stdout); }

void astra_println_i8(int8_t x) { astra_print_i8(x); putchar('\n'); }
void astra_println_i16(int16_t x) { astra_print_i16(x); putchar('\n'); }
void astra_println_i32(int32_t x) { astra_print_i32(x); putchar('\n'); }
void astra_println_i64(int64_t x) { astra_print_i64(x); putchar('\n'); }
void astra_println_u8(uint8_t x) { astra_print_u8(x); putchar('\n'); }
void astra_println_u16(uint16_t x) { astra_print_u16(x); putchar('\n'); }
void astra_println_u32(uint32_t x) { astra_print_u32(x); putchar('\n'); }
void astra_println_u64(uint64_t x) { astra_print_u64(x); putchar('\n'); }
void astra_println_usize(size_t x) { astra_print_usize(x); putchar('\n'); }
void astra_println_isize(intptr_t x) { astra_print_isize(x); putchar('\n'); }
void astra_println_f32(float x) { astra_print_f32(x); putchar('\n'); }
void astra_println_f64(double x) { astra_print_f64(x); putchar('\n'); }
void astra_println_bool(bool x) { astra_print_bool(x); putchar('\n'); }
void astra_println_str(NS_Str x) { astra_print_str(x); putchar('\n'); }
void astra_println_bytes(NS_Bytes x) { astra_print_bytes(x); putchar('\n'); }
void astra_println_unit(void) { astra_print_unit(); putchar('\n'); }

void astra_assert(bool cond) { if (!cond) { astra_panic("assertion failed"); } }

ASTRA_NORETURN static void astra_panic_owned(char* owned, const char* msg) {
    NS_Task_Core* task = astra_task_current;
    if (task) {

        size_t len = strlen(msg);
        char* copy = (char*)malloc(len + 1);
        if (copy) {
            memcpy(copy, msg, len + 1);
            free(task->panic_message);
            task->panic_message = copy;
        }
        task->panicked = true;
        free(owned);
        astra_task_abort();
    }
    fprintf(stderr, "panic: %s\n", msg);
    free(owned);
    abort();
}

void astra_panic(const char* msg) { astra_panic_owned(NULL, msg); }
void astra_abort(const char* msg) { fprintf(stderr, "abort: %s\n", msg); abort(); }

static const NS_Gc_Shape_Field astra_shape_fields_str[] = { { 0, &NS_SHAPE_LEAF } };
const NS_Gc_Shape NS_SHAPE_LEAF = { .kind = NS_SHAPE_KIND_LEAF };
const NS_Gc_Shape NS_SHAPE_STR = { .kind = NS_SHAPE_KIND_RECORD, .fields = astra_shape_fields_str, .field_count = 1 };
const NS_Gc_Shape NS_SHAPE_ARRAY_OF_STR = { .kind = NS_SHAPE_KIND_ARRAY, .elem_size = sizeof(NS_Str), .elem = &NS_SHAPE_STR };

typedef struct NS_Arena_Node { void* ptr; struct NS_Arena_Node* next; } NS_Arena_Node;

typedef struct { void* payload; size_t bytes; } NS_Gc_Dir_Entry;

typedef struct NS_Heap {
    NS_Arena_Node* arena_head;
    size_t arena_live;
    size_t arena_total;
    void** roots;
    size_t roots_cap;
    size_t root_count;
    void** pins;
    size_t pins_cap;
    size_t pins_len;
    size_t* scope_stack;
    size_t scope_cap;
    size_t scope_len;
    size_t bytes_since_gc;
    size_t collections;
    NS_Gc_Dir_Entry* dir;
    size_t dir_count;
    void** mark_stack;
    size_t mark_len;
    size_t mark_cap;

    bool immortal;
} NS_Heap;

static NS_Heap astra_main_heap;
static _Thread_local NS_Heap* astra_heap_current = NULL;

static _Thread_local int astra_gc_copy_depth = 0;

static NS_Heap* astra_heap(void) {
    if (!astra_heap_current) { astra_heap_current = &astra_main_heap; }
    return astra_heap_current;
}

static size_t astra_manual_live = 0;
static size_t astra_manual_total = 0;

static int astra_gc_stress = -1;

static void astra_gc_init_stress(void) {
    if (astra_gc_stress >= 0) { return; }
    const char* value = getenv("ASTRA_GC_STRESS");
    astra_gc_stress = (value && value[0] == '1' && value[1] == '\0') ? 1 : 0;
}

static void astra_gc_reserve(NS_Heap* h, size_t need) {
    if (need <= h->roots_cap) { return; }
    size_t cap = h->roots_cap == 0 ? 64 : h->roots_cap;
    while (cap < need) { cap *= 2; }
    void** fresh = (void**)realloc(h->roots, cap * sizeof(void*));
    if (!fresh) { astra_panic("out of memory"); }
    h->roots = fresh;
    h->roots_cap = cap;
}

size_t astra_gc_frame_enter(size_t slots) {
    NS_Heap* h = astra_heap();
    astra_gc_init_stress();
    size_t rf = h->root_count;
    astra_gc_reserve(h, rf + slots);
    memset(h->roots + rf, 0, slots * sizeof(void*));
    h->root_count = rf + slots;
    return rf;
}

void astra_gc_frame_exit(size_t rf) {
    NS_Heap* h = astra_heap();
    if (rf < h->root_count) { h->root_count = rf; }
}

void astra_gc_publish(size_t index, void* payload) {
    NS_Heap* h = astra_heap();

    if (index >= h->root_count) {
        astra_gc_reserve(h, index + 1);
        memset(
            h->roots + h->root_count,
            0,
            (index + 1 - h->root_count) * sizeof(void*)
        );
        h->root_count = index + 1;
    }
    h->roots[index] = payload;
}

size_t astra_gc_temp_enter(void) {
    NS_Heap* h = astra_heap();
    size_t index = h->root_count;
    astra_gc_reserve(h, index + 1);
    h->roots[index] = NULL;
    h->root_count = index + 1;
    return index;
}

void astra_gc_temp_exit(size_t index) {
    NS_Heap* h = astra_heap();
    if (index < h->root_count) { h->root_count = index; }
}

size_t astra_gc_root_count(void) { return astra_heap()->root_count; }
size_t astra_gc_collected_count(void) { return astra_heap()->collections; }

void astra_gc_pin(void* payload) {
    NS_Heap* h = astra_heap();
    if (!payload) { return; }
    if (h->pins_len == h->pins_cap) {
        size_t cap = h->pins_cap == 0 ? 32 : h->pins_cap * 2;
        void** fresh = (void**)realloc(h->pins, cap * sizeof(void*));
        if (!fresh) { astra_panic("out of memory"); }
        h->pins = fresh;
        h->pins_cap = cap;
    }
    h->pins[h->pins_len++] = payload;
}

size_t astra_gc_pin_count(void) { return astra_heap()->pins_len; }

size_t astra_gc_scope_enter(void) {
    NS_Heap* h = astra_heap();
    astra_gc_init_stress();
    size_t base = h->root_count;
    if (h->scope_len == h->scope_cap) {
        size_t cap = h->scope_cap == 0 ? 8 : h->scope_cap * 2;
        size_t* fresh = (size_t*)realloc(h->scope_stack, cap * sizeof(size_t));
        if (!fresh) { astra_panic("out of memory"); }
        h->scope_stack = fresh;
        h->scope_cap = cap;
    }
    h->scope_stack[h->scope_len++] = base;
    return base;
}

void astra_gc_root(size_t scope, const void* payload) {
    NS_Heap* h = astra_heap();
    if (!payload) { return; }
    (void)scope;
    astra_gc_reserve(h, h->root_count + 1);
    h->roots[h->root_count++] = (void*)payload;
}

void astra_gc_scope_exit(size_t scope) {
    NS_Heap* h = astra_heap();
    if (scope <= h->root_count) {
        h->root_count = scope;
    }
    if (h->scope_len > 0
        && h->scope_stack[h->scope_len - 1] == scope) {
        h->scope_len--;
    }
}

static NS_Gc_Head* astra_gc_head(void* payload) {
    return ((NS_Gc_Head*)payload) - 1;
}

static size_t astra_gc_poison_fills = 0;
static void astra_gc_poison(NS_Gc_Head* head) {
#ifndef NDEBUG
    memset((unsigned char*)(head + 1), 0xDD, head->bytes);
    astra_gc_poison_fills += 1;
#else
    (void)head;
#endif
}

void astra_gc_poison_payload(void* payload) {
    if (!payload) { return; }
    astra_gc_poison(astra_gc_head(payload));
}

size_t astra_gc_poison_count(void) { return astra_gc_poison_fills; }

static void astra_gc_collect_in(NS_Heap* h);

static void* astra_gc_alloc_in(NS_Heap* h, const NS_Gc_Shape* shape, size_t n) {

    astra_task_check();

    astra_gc_init_stress();
    if (astra_gc_stress && astra_gc_copy_depth == 0 && !h->immortal) { astra_gc_collect_in(h); }
    size_t bytes = sizeof(NS_Gc_Head) + (n == 0 ? 1 : n);
    NS_Gc_Head* head = (NS_Gc_Head*)calloc(bytes, 1);
    if (!head) { astra_panic("out of memory"); }
    head->marks = 0;
    head->bytes = n;
    head->shape = shape;
    NS_Arena_Node* node = (NS_Arena_Node*)malloc(sizeof(NS_Arena_Node));
    if (!node) { free(head); astra_panic("out of memory"); }
    node->ptr = (void*)(head + 1);
    node->next = h->arena_head;
    h->arena_head = node;
    h->arena_live += 1;
    h->arena_total += 1;
    h->bytes_since_gc += n;
    return (void*)(head + 1);
}

void* astra_gc_alloc(const NS_Gc_Shape* shape, size_t n) {
    return astra_gc_alloc_in(astra_heap(), shape, n);
}

static void astra_gc_free_in(NS_Heap* h, void* p) {
    if (!p) { return; }
    NS_Arena_Node** cur = &h->arena_head;
    while (*cur) {
        if ((*cur)->ptr == p) {
            NS_Arena_Node* dead = *cur;
            *cur = dead->next;
            astra_gc_poison(astra_gc_head(dead->ptr));
            free(astra_gc_head(dead->ptr));
            free(dead);
            h->arena_live -= 1;
            return;
        }
        cur = &(*cur)->next;
    }
    astra_panic("free of a pointer that is not a managed block");
}

void astra_gc_free(void* p) { astra_gc_free_in(astra_heap(), p); }

static void* astra_gc_realloc_in(NS_Heap* h, void* p, const NS_Gc_Shape* shape, size_t n) {
    if (!p) { return astra_gc_alloc_in(h, shape, n); }
    NS_Gc_Head* head = astra_gc_head(p);
    size_t old_bytes = head->bytes;
    h->bytes_since_gc += n;
    NS_Gc_Head* fresh = (NS_Gc_Head*)realloc(head, sizeof(NS_Gc_Head) + (n == 0 ? 1 : n));
    if (!fresh) { astra_panic("out of memory"); }
    fresh->bytes = n;
    if (n > old_bytes) {
        memset(((unsigned char*)(fresh + 1)) + old_bytes, 0, n - old_bytes);
    }
    void* payload = (void*)(fresh + 1);
    NS_Arena_Node** cur = &h->arena_head;
    while (*cur) {
        if ((*cur)->ptr == p) {
            (*cur)->ptr = payload;
            return payload;
        }
        cur = &(*cur)->next;
    }
    astra_panic("realloc of a pointer that is not a managed block");
}

void* astra_gc_realloc(void* p, const NS_Gc_Shape* shape, size_t n) {
    return astra_gc_realloc_in(astra_heap(), p, shape, n);
}

void* astra_manual_alloc(size_t n) {
    void* p = malloc(n == 0 ? 1 : n);
    if (!p) { astra_panic("out of memory"); }
    astra_manual_live += 1;
    astra_manual_total += 1;
    return p;
}

void astra_manual_free(void* p) {
    if (!p) { return; }
    astra_manual_live -= 1;
    free(p);
}

static int astra_gc_dir_cmp(const void* a, const void* b) {
    uintptr_t x = (uintptr_t)((const NS_Gc_Dir_Entry*)a)->payload;
    uintptr_t y = (uintptr_t)((const NS_Gc_Dir_Entry*)b)->payload;
    return x < y ? -1 : (x > y ? 1 : 0);
}

static void astra_gc_mark_push(NS_Heap* h, void* payload) {
    NS_Gc_Head* head = astra_gc_head(payload);
    if (head->marks != 0) { return; }
    head->marks = 1;
    if (h->mark_len == h->mark_cap) {
        size_t cap = h->mark_cap == 0 ? 64 : h->mark_cap * 2;
        void** fresh = (void**)realloc(h->mark_stack, cap * sizeof(void*));
        if (!fresh) { astra_panic("out of memory"); }
        h->mark_stack = fresh;
        h->mark_cap = cap;
    }
    h->mark_stack[h->mark_len++] = payload;
}

static void astra_gc_mark_pointer(NS_Heap* h, void* value) {
    if (!value) { return; }
    size_t lo = 0;
    size_t hi = h->dir_count;
    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        if ((uintptr_t)h->dir[mid].payload <= (uintptr_t)value) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if (lo == 0) { return; }
    const NS_Gc_Dir_Entry* entry = &h->dir[lo - 1];
    uintptr_t base = (uintptr_t)entry->payload;
    uintptr_t pointer = (uintptr_t)value;
    if (pointer == base || pointer < base + entry->bytes) {
        astra_gc_mark_push(h, entry->payload);
    }
}

static void astra_gc_trace(NS_Heap* h, const NS_Gc_Shape* shape, unsigned char* base, size_t bytes) {
    if (!shape) { return; }
    switch (shape->kind) {
    case NS_SHAPE_KIND_LEAF:
        break;
    case NS_SHAPE_KIND_ARRAY: {
        if (shape->elem_size == 0) { break; }
        size_t elements = bytes / shape->elem_size;
        for (size_t i = 0; i < elements; i++) {
            astra_gc_trace(h, shape->elem, base + i * shape->elem_size, 0);
        }
        break;
    }
    case NS_SHAPE_KIND_RECORD:
        for (size_t i = 0; i < shape->field_count; i++) {
            void* slot = *(void**)(base + shape->fields[i].offset);
            astra_gc_mark_pointer(h, slot);
        }
        break;
    case NS_SHAPE_KIND_VARIANT: {
        if (shape->variant_count == 0) { break; }
        int tag = *(const int*)(base + shape->tag_offset);
        if (tag >= 0 && (size_t)tag < shape->variant_count && shape->variants[tag]) {
            astra_gc_trace(h, shape->variants[tag], base, 0);
        }
        break;
    }
    }
}

static void astra_gc_collect_in(NS_Heap* h) {
    size_t count = h->arena_live;
    NS_Gc_Dir_Entry* dir = NULL;
    if (count > 0) {
        dir = (NS_Gc_Dir_Entry*)malloc(count * sizeof(NS_Gc_Dir_Entry));
        if (!dir) { astra_panic("out of memory"); }
        size_t i = 0;
        for (NS_Arena_Node* node = h->arena_head; node; node = node->next) {
            dir[i].payload = node->ptr;
            dir[i].bytes = astra_gc_head(node->ptr)->bytes;
            i++;
        }
        qsort(dir, count, sizeof(NS_Gc_Dir_Entry), astra_gc_dir_cmp);
    }
    h->dir = dir;
    h->dir_count = count;

    for (size_t i = 0; i < h->root_count; i++) {
        astra_gc_mark_pointer(h, h->roots[i]);
    }
    for (size_t i = 0; i < h->pins_len; i++) {
        astra_gc_mark_pointer(h, h->pins[i]);
    }
    while (h->mark_len > 0) {
        void* payload = h->mark_stack[--h->mark_len];
        NS_Gc_Head* head = astra_gc_head(payload);
        astra_gc_trace(h, head->shape, (unsigned char*)payload, head->bytes);
    }

    NS_Arena_Node** cur = &h->arena_head;
    while (*cur) {
        NS_Arena_Node* node = *cur;
        NS_Gc_Head* head = astra_gc_head(node->ptr);
        if (head->marks == 0) {
            *cur = node->next;
            astra_gc_poison(head);
            free(head);
            free(node);
            h->arena_live -= 1;
        } else {
            head->marks = 0;
            cur = &node->next;
        }
    }
    h->dir = NULL;
    h->dir_count = 0;
    free(dir);
    h->collections += 1;
    h->bytes_since_gc = 0;
}

void astra_gc_collect(void) { astra_gc_collect_in(astra_heap()); }

#define ASTRA_GC_DEFAULT_THRESHOLD (1024u * 1024u)
void astra_gc_maybe_collect(void) {
    astra_task_check();
    NS_Heap* h = astra_heap();
    astra_gc_init_stress();
    if (astra_gc_stress) { astra_gc_collect(); return; }
    if (h->bytes_since_gc >= ASTRA_GC_DEFAULT_THRESHOLD) {
        astra_gc_collect();
    }
}

#define ASTRA_REGION_ALIGN 8

NS_Region* astra_region_new(size_t bytes) {
    NS_Region* r = (NS_Region*)astra_gc_alloc(&NS_SHAPE_LEAF, sizeof(NS_Region));
    r->base = (uint8_t*)astra_manual_alloc(bytes == 0 ? 1 : bytes);
    r->cap = bytes;
    r->offset = 0;
    return r;
}

void* astra_region_alloc(NS_Region* r, size_t n) {
    if (!r || !r->base) { astra_panic("allocation from a freed region"); }
    size_t start = (r->offset + (ASTRA_REGION_ALIGN - 1)) & ~(size_t)(ASTRA_REGION_ALIGN - 1);
    if (n > r->cap || start > r->cap - n) { astra_panic("region capacity exceeded"); }
    r->offset = start + n;
    return r->base + start;
}

void astra_region_free(NS_Region* r) {
    if (!r || !r->base) { return; }
    astra_manual_free(r->base);
    r->base = NULL;
    r->cap = 0;
    r->offset = 0;
}

static void astra_heap_release_state(NS_Heap* h) {
    NS_Arena_Node* cur = h->arena_head;
    while (cur) {
        NS_Arena_Node* next = cur->next;
        free(astra_gc_head(cur->ptr));
        free(cur);
        cur = next;
    }
    h->arena_head = NULL;
    h->arena_live = 0;

    free(h->roots);
    h->roots = NULL;
    h->roots_cap = 0;
    h->root_count = 0;
    free(h->pins);
    h->pins = NULL;
    h->pins_cap = 0;
    h->pins_len = 0;
    free(h->scope_stack);
    h->scope_stack = NULL;
    h->scope_cap = 0;
    h->scope_len = 0;
    free(h->mark_stack);
    h->mark_stack = NULL;
    h->mark_len = 0;
    h->mark_cap = 0;
    h->bytes_since_gc = 0;
}

typedef struct {
#if defined(_WIN32)
    CRITICAL_SECTION mutex;
    CONDITION_VARIABLE cond;
#else
    pthread_mutex_t mutex;
    pthread_cond_t cond;
#endif
} NS_Channel_Sync;

static NS_Heap astra_channel_heap;
static NS_Channel_Sync astra_channel_shared_sync;

static void astra_channel_shared_lock(void);
static void astra_channel_shared_unlock(void);
static void astra_channel_sync_release_all(void);

void astra_runtime_shutdown(void) {
    NS_Heap* h = &astra_main_heap;
    astra_heap_release_state(h);

    astra_channel_shared_lock();
    astra_heap_release_state(&astra_channel_heap);
    astra_channel_sync_release_all();
    astra_channel_shared_unlock();

    astra_heap_current = &astra_main_heap;
}

size_t astra_arena_live_count(void) { return astra_heap()->arena_live; }
size_t astra_arena_total_count(void) { return astra_heap()->arena_total; }
size_t astra_manual_live_count(void) { return astra_manual_live; }
size_t astra_manual_total_count(void) { return astra_manual_total; }

NS_Heap* astra_heap_create(void) {
    NS_Heap* h = (NS_Heap*)calloc(1, sizeof(NS_Heap));
    if (!h) { astra_panic("out of memory"); }
    return h;
}

void astra_heap_bind(NS_Heap* h) { astra_heap_current = h; }

NS_Heap* astra_current_heap(void) { return astra_heap(); }

void* astra_gc_alloc_in_heap(NS_Heap* h, const NS_Gc_Shape* shape, size_t n) {
    return astra_gc_alloc_in(h, shape, n);
}

void astra_gc_root_in(NS_Heap* h, void* payload) {
    if (!payload) { return; }
    astra_gc_reserve(h, h->root_count + 1);
    h->roots[h->root_count++] = payload;
}

void astra_heap_release(NS_Heap* h) {
    if (!h || h == &astra_main_heap) { return; }
    astra_heap_release_state(h);
    if (astra_heap_current == h) { astra_heap_current = NULL; }
    free(h);
}

static void astra_task_trampoline(void* arg) {
    NS_Task_Core* task = (NS_Task_Core*)arg;
    astra_heap_bind(task->heap);
    astra_task_current = task;
    task->entry(task->closure);
    task->has_value = true;
    task->finished = true;
    astra_task_current = NULL;
}

#if defined(_WIN32)
static DWORD WINAPI astra_task_thread_entry(LPVOID arg) {
    astra_task_trampoline(arg);
    return 0;
}
#else
static void* astra_task_thread_entry(void* arg) {
    astra_task_trampoline(arg);
    return NULL;
}
#endif

void astra_task_start(NS_Task_Core* task) {
    if (!task->heap) { task->heap = astra_heap_create(); }
    task->started = true;

    if (astra_task_current) { astra_task_current->child_count += 1; }
#if defined(_WIN32)
    HANDLE handle = CreateThread(NULL, 0, astra_task_thread_entry, task, 0, NULL);
    if (!handle) { astra_panic("failed to start task"); }
    task->handle = (uintptr_t)handle;
#else
    pthread_t handle;
    if (pthread_create(&handle, NULL, astra_task_thread_entry, task) != 0) {
        astra_panic("failed to start task");
    }
    task->handle = (uintptr_t)handle;
#endif
}

bool astra_task_join(NS_Task_Core* task) {
    if (task->started) {
#if defined(_WIN32)
        WaitForSingleObject((HANDLE)task->handle, INFINITE);
        CloseHandle((HANDLE)task->handle);
#else
        pthread_join((pthread_t)task->handle, NULL);
#endif
        task->handle = 0;
        task->started = false;
        task->finished = true;
    }

    return task->has_value && !task->panicked;
}

void astra_task_heap_free(NS_Task_Core* task) {
    if (task->panic_message) {
        free(task->panic_message);
        task->panic_message = NULL;
    }
    if (!task->heap) { return; }

    if ((task->canceled || task->panicked) && task->child_count > 0) {
        task->heap = NULL;
        return;
    }
    astra_heap_release(task->heap);
    task->heap = NULL;
}

void astra_task_cancel(NS_Task_Core* task) {

    if (task && task->started && !task->finished) { task->canceled = true; }
}

bool astra_task_canceled(void) {
    return astra_task_current && astra_task_current->canceled;
}

ASTRA_NORETURN static void astra_task_abort(void) {
    if (astra_task_current) { astra_task_current->finished = true; }
#if defined(_WIN32)
    ExitThread(0);
#else
    pthread_exit(NULL);
#endif
}

static ASTRA_NORETURN void astra_task_error(const char* detail) {
    static const char prefix[] = "TaskError: ";
    const size_t prefix_len = sizeof(prefix) - 1;
    if (detail && strncmp(detail, prefix, prefix_len) == 0) {
        astra_panic(detail);
    }
    const size_t detail_len = detail ? strlen(detail) : 0;
    char* message = (char*)malloc(prefix_len + detail_len + 1);
    if (!message) { astra_panic(prefix); }
    memcpy(message, prefix, prefix_len);
    if (detail_len > 0) { memcpy(message + prefix_len, detail, detail_len); }
    message[prefix_len + detail_len] = '\0';
    astra_panic_owned(message, message);
}

ASTRA_NORETURN void astra_task_raise(const NS_Task_Core* task) {

    if (task->panicked) {
        astra_task_error(task->panic_message ? task->panic_message : "child task panicked");
    }
    if (task->canceled) {
        astra_task_error("task was cancelled");
    }
    astra_task_error("task failed without a value");
}

void astra_task_check(void) {

    if (astra_gc_copy_depth > 0) { return; }
    NS_Task_Core* task = astra_task_current;
    if (task && task->canceled) { astra_task_abort(); }
}

static NS_Arena_Node* astra_gc_block_containing(NS_Heap* h, const void* pointer, size_t* offset) {
    uintptr_t target = (uintptr_t)pointer;
    for (NS_Arena_Node* node = h->arena_head; node; node = node->next) {
        uintptr_t base = (uintptr_t)node->ptr;
        size_t bytes = astra_gc_head(node->ptr)->bytes;
        if (bytes == 0) { bytes = 1; }
        if (target >= base && target < base + bytes) {
            *offset = (size_t)(target - base);
            return node;
        }
    }
    return NULL;
}

static void astra_gc_copy_value(NS_Heap* dst, NS_Heap* src, const NS_Gc_Shape* shape, size_t bytes, void* dst_value, const void* src_value);

static void* astra_gc_copy_block(NS_Heap* dst, NS_Heap* src, const NS_Gc_Shape* shape, size_t n, const void* payload) {
    void* copy = astra_gc_alloc_in(dst, shape, n);
    memcpy(copy, payload, n);
    astra_gc_copy_value(dst, src, shape, n, copy, payload);
    return copy;
}

static void* astra_gc_copy_pointer(NS_Heap* dst, NS_Heap* src, void* pointer) {
    if (!pointer) { return NULL; }
    size_t offset = 0;
    NS_Arena_Node* node = astra_gc_block_containing(src, pointer, &offset);
    if (!node) { return pointer; }
    NS_Gc_Head* head = astra_gc_head(node->ptr);
    void* copy = astra_gc_copy_block(dst, src, head->shape, head->bytes, node->ptr);
    return (char*)copy + offset;
}

static void astra_gc_copy_value(NS_Heap* dst, NS_Heap* src, const NS_Gc_Shape* shape, size_t bytes, void* dst_value, const void* src_value) {
    if (!shape) { return; }
    switch (shape->kind) {
    case NS_SHAPE_KIND_LEAF:
        break;
    case NS_SHAPE_KIND_ARRAY: {
        if (shape->elem_size == 0) { break; }
        size_t elements = bytes / shape->elem_size;
        for (size_t i = 0; i < elements; i++) {
            astra_gc_copy_value(dst, src, shape->elem, 0,
                (char*)dst_value + i * shape->elem_size,
                (const char*)src_value + i * shape->elem_size);
        }
        break;
    }
    case NS_SHAPE_KIND_RECORD:
        for (size_t i = 0; i < shape->field_count; i++) {
            void** dst_slot = (void**)((char*)dst_value + shape->fields[i].offset);
            void* src_pointer = *(void**)((const char*)src_value + shape->fields[i].offset);
            *dst_slot = astra_gc_copy_pointer(dst, src, src_pointer);
        }
        break;
    case NS_SHAPE_KIND_VARIANT: {
        if (shape->variant_count == 0) { break; }
        int tag = *(const int*)((const char*)src_value + shape->tag_offset);
        if (tag >= 0 && (size_t)tag < shape->variant_count && shape->variants[tag]) {
            astra_gc_copy_value(dst, src, shape->variants[tag], 0, dst_value, src_value);
        }
        break;
    }
    }
}

void astra_gc_copy_into(NS_Heap* dst, NS_Heap* src, const NS_Gc_Shape* shape, size_t n, void* dst_value, const void* src_value) {
    if (dst_value != src_value) { memcpy(dst_value, src_value, n); }
    astra_gc_copy_depth++;
    astra_gc_copy_value(dst, src, shape, n, dst_value, src_value);
    astra_gc_copy_depth--;
}

static void astra_gc_free_pointer_in(NS_Heap* h, void* pointer);

static void astra_gc_free_value_in(NS_Heap* h, const NS_Gc_Shape* shape, size_t bytes, void* value) {
    if (!shape || !value) { return; }
    switch (shape->kind) {
    case NS_SHAPE_KIND_LEAF:
        break;
    case NS_SHAPE_KIND_ARRAY: {
        if (shape->elem_size == 0) { break; }
        size_t elements = bytes / shape->elem_size;
        for (size_t i = 0; i < elements; i++) {
            astra_gc_free_value_in(h, shape->elem, 0,
                (char*)value + i * shape->elem_size);
        }
        break;
    }
    case NS_SHAPE_KIND_RECORD:
        for (size_t i = 0; i < shape->field_count; i++) {
            astra_gc_free_pointer_in(h, *(void**)((char*)value + shape->fields[i].offset));
        }
        break;
    case NS_SHAPE_KIND_VARIANT: {
        if (shape->variant_count == 0) { break; }
        int tag = *(int*)((char*)value + shape->tag_offset);
        if (tag >= 0 && (size_t)tag < shape->variant_count && shape->variants[tag]) {
            astra_gc_free_value_in(h, shape->variants[tag], 0, value);
        }
        break;
    }
    }
}

static void astra_gc_free_pointer_in(NS_Heap* h, void* pointer) {
    if (!pointer) { return; }
    size_t offset = 0;
    NS_Arena_Node* node = astra_gc_block_containing(h, pointer, &offset);
    if (!node) { return; }
    NS_Gc_Head* head = astra_gc_head(node->ptr);
    astra_gc_free_value_in(h, head->shape, head->bytes, node->ptr);
    astra_gc_free_in(h, node->ptr);
}

NS_Array astra_array_new_lit(size_t n, size_t elem_size, const void* src, const NS_Gc_Shape* shape) {
    NS_Array arr;
    arr.cap = n == 0 ? 1 : n;
    arr.len = n;
    arr.data = astra_gc_alloc(shape, arr.cap * (elem_size == 0 ? 1 : elem_size));
    if (n > 0) { memcpy(arr.data, src, n * elem_size); }
    return arr;
}

void astra_array_push(NS_Array* arr, size_t elem_size, const void* value, const NS_Gc_Shape* shape) {
    if (arr->len >= arr->cap) {
        size_t new_cap = arr->cap == 0 ? 4 : arr->cap * 2;
        arr->data = astra_gc_realloc(arr->data, shape, new_cap * (elem_size == 0 ? 1 : elem_size));
        arr->cap = new_cap;
    }
    memcpy((char*)arr->data + arr->len * elem_size, value, elem_size);
    arr->len += 1;
}

void* astra_array_index(NS_Array* arr, size_t i, size_t elem_size) {
    if (i >= arr->len) { astra_panic("array index out of bounds"); }
    return (char*)arr->data + i * elem_size;
}

size_t astra_str_len(NS_Str s) { return s.len; }
bool astra_str_is_empty(NS_Str s) { return s.len == 0; }
bool astra_str_eq(NS_Str a, NS_Str b) {
    if (a.len != b.len) return false;
    if (a.len == 0) return true;
    return memcmp(a.data, b.data, a.len) == 0;
}

int astra_str_cmp(NS_Str a, NS_Str b) {
    size_t min_len = a.len < b.len ? a.len : b.len;
    if (min_len > 0) {
        int r = memcmp(a.data, b.data, min_len);
        if (r != 0) return r;
    }
    return a.len < b.len ? -1 : (a.len > b.len ? 1 : 0);
}

static bool astra_utf8_boundary(const char* data, size_t pos, size_t len) {
    if (pos == len) return true;
    if (pos > len) return false;
    return ((unsigned char)data[pos] & 0xC0) != 0x80;
}

bool astra_str_slice_try(NS_Str s, size_t start, size_t end, NS_Str* out) {
    if (start > end || end > s.len) return false;
    if (!astra_utf8_boundary(s.data, start, s.len)) return false;
    if (!astra_utf8_boundary(s.data, end, s.len)) return false;
    out->data = s.data + start;
    out->len = end - start;
    return true;
}

bool astra_str_to_int_try(NS_Str s, int32_t* out) {
    size_t i = 0;
    bool negative = false;
    if (i < s.len && (s.data[i] == '+' || s.data[i] == '-')) {
        negative = s.data[i] == '-';
        i += 1;
    }
    if (i >= s.len) return false;
    int64_t value = 0;
    for (; i < s.len; i += 1) {
        unsigned char digit = (unsigned char)s.data[i];
        if (digit < '0' || digit > '9') return false;
        value = value * 10 + (int64_t)(digit - '0');

        if (value > 2147483648LL) return false;
    }
    if (negative) value = -value;
    if (value < -2147483648LL || value > 2147483647LL) return false;
    *out = (int32_t)value;
    return true;
}

static size_t astra_utf8_width(const char* data, size_t len) {
    unsigned char lead = (unsigned char)data[0];
    if (lead < 0x80) return 1;
    size_t width = lead >= 0xF0 ? 4 : lead >= 0xE0 ? 3 : lead >= 0xC0 ? 2 : 0;
    if (width == 0 || width > len) return 0;
    return width;
}

size_t astra_str_char_count(NS_Str s) {
    size_t count = 0;
    size_t i = 0;
    while (i < s.len) {
        size_t width = astra_utf8_width(s.data + i, s.len - i);
        if (width == 0) width = 1;
        count += 1;
        i += width;
    }
    return count;
}

bool astra_str_char_at(NS_Str s, size_t index, NS_Str* out) {
    size_t count = 0;
    size_t i = 0;
    while (i < s.len) {
        size_t width = astra_utf8_width(s.data + i, s.len - i);
        if (width == 0) width = 1;
        if (count == index) {
            out->data = s.data + i;
            out->len = width;
            return true;
        }
        count += 1;
        i += width;
    }
    return false;
}

NS_Array astra_str_bytes(NS_Str s) {
    NS_Array arr;
    arr.data = astra_gc_alloc(&NS_SHAPE_LEAF, s.len == 0 ? 1 : s.len);
    arr.len = s.len;
    arr.cap = s.len;
    if (s.len > 0) memcpy(arr.data, s.data, s.len);
    return arr;
}

NS_Array astra_str_chars(NS_Str s) {
    NS_Array arr;
    arr.data = NULL;
    arr.len = 0;
    arr.cap = 0;
    size_t i = 0;
    while (i < s.len) {
        size_t width = astra_utf8_width(s.data + i, s.len - i);
        if (width == 0) width = 1;
        NS_Str part;
        part.data = s.data + i;
        part.len = width;
        astra_array_push(&arr, sizeof(NS_Str), &part, &NS_SHAPE_ARRAY_OF_STR);
        i += width;
    }
    return arr;
}

NS_Bytes astra_bytes_new(size_t len) {
    NS_Bytes b;

    b.data = (const uint8_t*)astra_gc_alloc(&NS_SHAPE_LEAF, len == 0 ? 1 : len);
    if (b.data == NULL) astra_panic("out of memory");
    b.len = len;
    return b;
}

bool astra_bytes_eq(NS_Bytes a, NS_Bytes b) {
    if (a.len != b.len) return false;
    if (a.len == 0) return true;
    return memcmp(a.data, b.data, a.len) == 0;
}

int astra_bytes_cmp(NS_Bytes a, NS_Bytes b) {
    size_t min_len = a.len < b.len ? a.len : b.len;
    if (min_len > 0) {
        int r = memcmp(a.data, b.data, min_len);
        if (r != 0) return r;
    }
    return a.len < b.len ? -1 : (a.len > b.len ? 1 : 0);
}

uint8_t astra_bytes_get(NS_Bytes b, size_t i) {
    if (i >= b.len) astra_panic("bytes index out of bounds");
    return b.data[i];
}

void astra_bytes_set(NS_Bytes* b, size_t i, uint8_t value) {
    if (i >= b->len) astra_panic("byte index out of bounds");
    ((uint8_t*)b->data)[i] = value;
}

NS_Bytes astra_bytes_slice(NS_Bytes b, size_t start, size_t end) {
    if (start > end || end > b.len) astra_panic("bytes slice is out of bounds");
    NS_Bytes res;
    res.data = b.data + start;
    res.len = end - start;
    return res;
}

static bool astra_keys_equal(const void* a, const void* b, size_t size, bool is_str) {
    if (is_str) {
        const NS_Str* sa = (const NS_Str*)a;
        const NS_Str* sb = (const NS_Str*)b;
        if (sa->len != sb->len) return false;
        return memcmp(sa->data, sb->data, sa->len) == 0;
    }
    return memcmp(a, b, size) == 0;
}

NS_Map astra_map_new(size_t key_size, size_t val_size, bool key_is_str, const NS_Gc_Shape* key_shape, const NS_Gc_Shape* val_shape) {
    NS_Map m;
    m.keys = NULL;
    m.values = NULL;
    m.len = 0;
    m.cap = 0;
    m.key_size = key_size;
    m.val_size = val_size;
    m.key_is_str = key_is_str;
    m.key_shape = key_shape;
    m.val_shape = val_shape;
    return m;
}

void astra_map_set(NS_Map* m, const void* key, const void* val) {
    if (!m) return;
    char* k_bytes = (char*)m->keys;
    char* v_bytes = (char*)m->values;
    for (size_t i = 0; i < m->len; i++) {
        if (astra_keys_equal(k_bytes + i * m->key_size, key, m->key_size, m->key_is_str)) {
            memcpy(v_bytes + i * m->val_size, val, m->val_size);
            return;
        }
    }
    if (m->len >= m->cap) {
        size_t new_cap = m->cap == 0 ? 8 : m->cap * 2;

        size_t root = astra_gc_temp_enter();
        m->keys = astra_gc_realloc(m->keys, m->key_shape, new_cap * (m->key_size == 0 ? 1 : m->key_size));
        astra_gc_publish(root, m->keys);
        m->values = astra_gc_realloc(m->values, m->val_shape, new_cap * (m->val_size == 0 ? 1 : m->val_size));
        astra_gc_temp_exit(root);
        m->cap = new_cap;
    }
    memcpy((char*)m->keys + m->len * m->key_size, key, m->key_size);
    memcpy((char*)m->values + m->len * m->val_size, val, m->val_size);
    m->len++;
}

void* astra_map_get(const NS_Map* m, const void* key) {
    if (!m || m->len == 0) return NULL;
    char* k_bytes = (char*)m->keys;
    char* v_bytes = (char*)m->values;
    for (size_t i = 0; i < m->len; i++) {
        if (astra_keys_equal(k_bytes + i * m->key_size, key, m->key_size, m->key_is_str)) {
            return v_bytes + i * m->val_size;
        }
    }
    return NULL;
}

bool astra_map_contains(const NS_Map* m, const void* key) {
    return astra_map_get(m, key) != NULL;
}

size_t astra_map_len(const NS_Map* m) {
    return m ? m->len : 0;
}

void* astra_map_key_at(const NS_Map* m, size_t i) {
    if (!m || i >= m->len) astra_panic("map key index out of bounds");
    return (char*)m->keys + (i * m->key_size);
}

#define ASTRA_CHANNEL_CAP 16
#define ASTRA_CHANNEL_DEFAULT_TIMEOUT 5.0

static double astra_now_seconds(void) {
#if defined(_WIN32)
    return (double)GetTickCount64() / 1000.0;
#else
    struct timespec ts;
    timespec_get(&ts, TIME_UTC);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1000000000.0;
#endif
}

static double astra_channel_timeout_seconds(void) {
    static double seconds = -1.0;
    if (seconds >= 0.0) { return seconds; }
    seconds = ASTRA_CHANNEL_DEFAULT_TIMEOUT;
    const char* raw = getenv("ASTRA_CHANNEL_DEADLOCK_TIMEOUT");
    if (raw && *raw) {
        char* end = NULL;
        double parsed = strtod(raw, &end);
        if (end != raw && parsed > 0.0) { seconds = parsed; }
    }
    return seconds;
}

static NS_Channel_Sync* astra_channel_sync_new(void) {
    NS_Channel_Sync* sync = (NS_Channel_Sync*)calloc(1, sizeof(NS_Channel_Sync));
    if (!sync) { astra_panic("out of memory"); }
#if defined(_WIN32)
    InitializeCriticalSection(&sync->mutex);
    InitializeConditionVariable(&sync->cond);
#else
    pthread_mutex_init(&sync->mutex, NULL);
    pthread_cond_init(&sync->cond, NULL);
#endif
    return sync;
}

static void astra_channel_lock(NS_Channel_Sync* sync) {
#if defined(_WIN32)
    EnterCriticalSection(&sync->mutex);
#else
    pthread_mutex_lock(&sync->mutex);
#endif
}

static void astra_channel_unlock(NS_Channel_Sync* sync) {
#if defined(_WIN32)
    LeaveCriticalSection(&sync->mutex);
#else
    pthread_mutex_unlock(&sync->mutex);
#endif
}

static void astra_channel_signal(NS_Channel_Sync* sync) {
#if defined(_WIN32)
    WakeConditionVariable(&sync->cond);
#else
    pthread_cond_signal(&sync->cond);
#endif
}

static void astra_channel_broadcast(NS_Channel_Sync* sync) {
#if defined(_WIN32)
    WakeAllConditionVariable(&sync->cond);
#else
    pthread_cond_broadcast(&sync->cond);
#endif
}

#define ASTRA_CHANNEL_CANCEL_SLICE 0.05
static bool astra_channel_wait(NS_Channel_Sync* sync, double seconds, bool* timed_out) {
#if defined(_WIN32)
    DWORD milliseconds = (DWORD)(seconds * 1000.0);
    if (milliseconds == 0) { milliseconds = 1; }
    if (SleepConditionVariableCS(&sync->cond, &sync->mutex, milliseconds)) {
        *timed_out = false;
        return true;
    }
    *timed_out = GetLastError() == ERROR_TIMEOUT;
    return false;
#else
    struct timespec deadline;
    timespec_get(&deadline, TIME_UTC);
    time_t whole = (time_t)seconds;
    long nanos = (long)((seconds - (double)whole) * 1000000000.0);
    deadline.tv_sec += whole;
    deadline.tv_nsec += nanos;
    if (deadline.tv_nsec >= 1000000000L) {
        deadline.tv_sec += 1;
        deadline.tv_nsec -= 1000000000L;
    }
    int rc = pthread_cond_timedwait(&sync->cond, &sync->mutex, &deadline);
    *timed_out = rc == ETIMEDOUT;
    return rc == 0;
#endif
}

#if defined(_WIN32)
static INIT_ONCE astra_channel_shared_once = INIT_ONCE_STATIC_INIT;
static BOOL CALLBACK astra_channel_shared_init(PINIT_ONCE once, PVOID param, PVOID* context) {
    (void)once;
    (void)param;
    (void)context;
    astra_channel_heap.immortal = true;
    InitializeCriticalSection(&astra_channel_shared_sync.mutex);
    InitializeConditionVariable(&astra_channel_shared_sync.cond);
    return TRUE;
}
#else
static pthread_once_t astra_channel_shared_once = PTHREAD_ONCE_INIT;
static void astra_channel_shared_init(void) {
    astra_channel_heap.immortal = true;
    pthread_mutex_init(&astra_channel_shared_sync.mutex, NULL);
    pthread_cond_init(&astra_channel_shared_sync.cond, NULL);
}
#endif

static void astra_channel_shared_ensure(void) {
#if defined(_WIN32)
    InitOnceExecuteOnce(&astra_channel_shared_once, astra_channel_shared_init, NULL, NULL);
#else
    pthread_once(&astra_channel_shared_once, astra_channel_shared_init);
#endif
}

static void astra_channel_shared_lock(void) {
    astra_channel_shared_ensure();
    astra_channel_lock(&astra_channel_shared_sync);
}

static void astra_channel_shared_unlock(void) {
    astra_channel_unlock(&astra_channel_shared_sync);
}

typedef struct NS_Channel_Sync_Node {
    NS_Channel_Sync* sync;
    struct NS_Channel_Sync_Node* next;
} NS_Channel_Sync_Node;

static NS_Channel_Sync_Node* astra_channel_sync_registry = NULL;

static void astra_channel_sync_register(NS_Channel_Sync* sync) {
    NS_Channel_Sync_Node* node = (NS_Channel_Sync_Node*)calloc(1, sizeof(NS_Channel_Sync_Node));
    if (!node) { astra_panic("out of memory"); }
    node->sync = sync;
    node->next = astra_channel_sync_registry;
    astra_channel_sync_registry = node;
}

static void astra_channel_sync_release_all(void) {
    NS_Channel_Sync_Node* node = astra_channel_sync_registry;
    while (node) {
        NS_Channel_Sync_Node* next = node->next;
#if defined(_WIN32)
        DeleteCriticalSection(&node->sync->mutex);
#else
        pthread_mutex_destroy(&node->sync->mutex);
        pthread_cond_destroy(&node->sync->cond);
#endif
        free(node->sync);
        free(node);
        node = next;
    }
    astra_channel_sync_registry = NULL;
}

size_t astra_channel_heap_live_count(void) {
    astra_channel_shared_lock();
    size_t live = astra_channel_heap.arena_live;
    astra_channel_shared_unlock();
    return live;
}

NS_Channel* astra_channel_new(size_t elem_size, const NS_Gc_Shape* elem_shape, const NS_Gc_Shape* handle_shape, const NS_Gc_Shape* data_shape) {
    astra_task_check();
    astra_channel_shared_lock();

    astra_gc_copy_depth++;
    NS_Channel* c = (NS_Channel*)astra_gc_alloc_in(&astra_channel_heap, handle_shape, sizeof(NS_Channel));
    c->data = astra_gc_alloc_in(&astra_channel_heap, data_shape, elem_size * ASTRA_CHANNEL_CAP);
    c->sync = astra_channel_sync_new();
    astra_channel_sync_register((NS_Channel_Sync*)c->sync);
    astra_gc_copy_depth--;
    astra_channel_shared_unlock();
    c->len = 0;
    c->cap = ASTRA_CHANNEL_CAP;
    c->elem_size = elem_size;
    c->elem_shape = elem_shape;
    c->closed = false;
    return c;
}

bool astra_channel_send(NS_Channel* c, const void* value) {
    if (!c) return false;
    astra_task_check();
    NS_Channel_Sync* sync = (NS_Channel_Sync*)c->sync;
    double deadline = astra_now_seconds() + astra_channel_timeout_seconds();
    astra_channel_lock(sync);
    while (!c->closed && c->len >= c->cap) {
        if (astra_task_canceled()) {
            astra_channel_unlock(sync);
            astra_task_check();
        }
        double left = deadline - astra_now_seconds();
        if (left <= 0.0) {
            astra_channel_unlock(sync);
            astra_panic("channel deadlock: send on a full channel");
        }
        if (astra_task_current && left > ASTRA_CHANNEL_CANCEL_SLICE) {
            left = ASTRA_CHANNEL_CANCEL_SLICE;
        }
        bool timed_out = false;
        astra_channel_wait(sync, left, &timed_out);
        (void)timed_out;
    }
    if (c->closed) {
        astra_channel_unlock(sync);
        return false;
    }

    void* slot = (char*)c->data + (c->len * c->elem_size);
    astra_channel_shared_lock();
    astra_gc_copy_into(&astra_channel_heap, astra_heap(), c->elem_shape, c->elem_size, slot, value);
    astra_channel_shared_unlock();
    c->len++;
    astra_channel_signal(sync);
    astra_channel_unlock(sync);
    return true;
}

bool astra_channel_receive(NS_Channel* c, void* out) {
    if (!c) return false;
    astra_task_check();
    NS_Channel_Sync* sync = (NS_Channel_Sync*)c->sync;
    double deadline = astra_now_seconds() + astra_channel_timeout_seconds();
    astra_channel_lock(sync);
    while (c->len == 0 && !c->closed) {
        if (astra_task_canceled()) {
            astra_channel_unlock(sync);
            astra_task_check();
        }
        double left = deadline - astra_now_seconds();
        if (left <= 0.0) {
            astra_channel_unlock(sync);
            astra_panic("channel deadlock: receive on an empty channel");
        }
        if (astra_task_current && left > ASTRA_CHANNEL_CANCEL_SLICE) {
            left = ASTRA_CHANNEL_CANCEL_SLICE;
        }
        bool timed_out = false;
        astra_channel_wait(sync, left, &timed_out);
        (void)timed_out;
    }
    if (c->len == 0) {
        astra_channel_unlock(sync);
        return false;
    }

    void* slot = c->data;
    astra_channel_shared_lock();
    astra_gc_copy_into(astra_heap(), &astra_channel_heap, c->elem_shape, c->elem_size, out, slot);
    astra_gc_free_value_in(&astra_channel_heap, c->elem_shape, c->elem_size, slot);
    memmove(slot, (char*)slot + c->elem_size, (c->len - 1) * c->elem_size);
    astra_channel_shared_unlock();
    c->len--;
    astra_channel_signal(sync);
    astra_channel_unlock(sync);
    return true;
}

void astra_channel_close(NS_Channel* c) {
    if (!c) return;
    NS_Channel_Sync* sync = (NS_Channel_Sync*)c->sync;
    astra_channel_lock(sync);
    c->closed = true;
    astra_channel_broadcast(sync);
    astra_channel_unlock(sync);
}

#define ASTRA_WRAP(name, type, utype, op) \
    type name(type a, type b) { return (type)((utype)a op (utype)b); }

ASTRA_WRAP(astra_add_i8, int8_t, uint8_t, +)
ASTRA_WRAP(astra_add_i16, int16_t, uint16_t, +)
ASTRA_WRAP(astra_add_i32, int32_t, uint32_t, +)
ASTRA_WRAP(astra_add_i64, int64_t, uint64_t, +)
ASTRA_WRAP(astra_sub_i8, int8_t, uint8_t, -)
ASTRA_WRAP(astra_sub_i16, int16_t, uint16_t, -)
ASTRA_WRAP(astra_sub_i32, int32_t, uint32_t, -)
ASTRA_WRAP(astra_sub_i64, int64_t, uint64_t, -)
ASTRA_WRAP(astra_mul_i8, int8_t, uint8_t, *)
ASTRA_WRAP(astra_mul_i16, int16_t, uint16_t, *)
ASTRA_WRAP(astra_mul_i32, int32_t, uint32_t, *)
ASTRA_WRAP(astra_mul_i64, int64_t, uint64_t, *)
"#;

pub fn host_shim() -> String {
    crate::cstrip::strip_comments(&format!("{RUNTIME_HEADER}\n{HOST_SHIM}"))
}
