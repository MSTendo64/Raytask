// Example native C library for [link: "examples/native/add.c"]

#include <stdint.h>

int native_add(int a, int b) {
    return a + b;
}

double native_hypot(double x, double y) {
    return x * x + y * y;
}
