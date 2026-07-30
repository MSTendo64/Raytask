#include <stdio.h>

int main(int argc, char **argv) {
    printf("hello from vendored tcc");
    if (argc > 1) {
        printf(": %s", argv[1]);
    }
    printf("\n");
    return 0;
}
