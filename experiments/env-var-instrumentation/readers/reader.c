#include <stdio.h>
#include <stdlib.h>
int main(void) {
    const char *v = getenv("MY_SECRET_TOKEN");
    printf("MY_SECRET_TOKEN=%s\n", v ? v : "(unset)");
    return 0;
}
