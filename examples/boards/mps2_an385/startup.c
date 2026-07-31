/* Cortex-M3 startup for ARM MPS2 AN385 / QEMU. */
#include <stdint.h>

extern uint32_t _estack;
extern uint32_t _sidata, _sdata, _edata, _sbss, _ebss;
extern void Main(void);

void Reset_Handler(void);
void Default_Handler(void);

void Default_Handler(void) {
    for (;;) {
    }
}

void Reset_Handler(void) {
    uint32_t *src = &_sidata;
    uint32_t *dst = &_sdata;
    while (dst < &_edata) {
        *dst++ = *src++;
    }
    for (dst = &_sbss; dst < &_ebss; ++dst) {
        *dst = 0;
    }
    Main();
    for (;;) {
    }
}

typedef void (*vector_fn)(void);

__attribute__((section(".isr_vector"), used))
const vector_fn g_vectors[] = {
    (vector_fn)(&_estack),
    Reset_Handler,
    Default_Handler,
    Default_Handler,
    Default_Handler,
    Default_Handler,
    Default_Handler,
    0,
    0,
    0,
    0,
    Default_Handler,
    Default_Handler,
    0,
    Default_Handler,
    Default_Handler,
};
