/* bitbang_ft2232.c

   Output some flickering in bitbang mode to the FT2232

   Thanks to max@koeln.ccc.de for fixing and extending
   the example for the second channel.

   This program is distributed under the GPL, version 2
*/

#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <ftdi.h>

int main(int argc, char **argv)
{
    struct ftdi_context *ftdi, *ftdi2;
    unsigned char buf[1];
    int f,i;

    // Init 1. channel
    if ((ftdi = ftdi_new()) == 0)
    {
        fprintf(stderr, "ftdi_new failed\n");
        return EXIT_FAILURE;
    }

    ftdi_set_interface(ftdi, INTERFACE_A);
    f = ftdi_usb_open(ftdi, 0x0403, 0x6001);
    if (f < 0 && f != -5)
    {
        fprintf(stderr, "unable to open ftdi device: %d (%s)\n", f, ftdi_get_error_string(ftdi));
        ftdi_free(ftdi);
        exit(-1);
    }
    printf("ftdi open succeeded(channel 1): %d\n",f);

    printf("enabling bitbang mode(channel 1)\n");
    ftdi_set_bitmode(ftdi, 0xFF, BITMODE_BITBANG);

    // Init 2. channel
    if ((ftdi2 = ftdi_new()) == 0)
    {
        fprintf(stderr, "ftdi_new failed\n");
        return EXIT_FAILURE;
    }
    ftdi_set_interface(ftdi2, INTERFACE_B);
    f = ftdi_usb_open(ftdi2, 0x0403, 0x6001);
    if (f < 0 && f != -5)
    {
        fprintf(stderr, "unable to open ftdi device: %d (%s)\n", f, ftdi_get_error_string(ftdi2));
        ftdi_free(ftdi2);
        exit(-1);
    }
    printf("ftdi open succeeded(channel 2): %d\n",f);

    printf("enabling bitbang mode (channel 2)\n");
    ftdi_set_bitmode(ftdi2, 0xFF, BITMODE_BITBANG);

    // Write data
    printf("startloop\n");
    for (i = 0; i < 23; i++)
    {
        buf[0] =  0x1;
        printf("porta: %02i: 0x%02x \n",i,buf[0]);
        f = ftdi_write_data(ftdi, buf, 1);
        if (f < 0)
            fprintf(stderr,"write failed on channel 1 for 0x%x, error %d (%s)\n", buf[0], f, ftdi_get_error_string(ftdi));
        usleep(1 * 1000000);

        buf[0] =  0x2;
        printf("porta: %02i: 0x%02x \n",i,buf[0]);
        f = ftdi_write_data(ftdi, buf, 1);
        if (f < 0)
            fprintf(stderr,"write failed on channel 1 for 0x%x, error %d (%s)\n", buf[0], f, ftdi_get_error_string(ftdi));
        usleep(1 * 1000000);

        buf[0] =  0x1;
        printf("portb: %02i: 0x%02x \n",i,buf[0]);
        f = ftdi_write_data(ftdi2, buf, 1);
        if (f < 0)
            fprintf(stderr,"write failed on channel 2 for 0x%x, error %d (%s)\n", buf[0], f, ftdi_get_error_string(ftdi2));
        usleep(1 * 1000000);

        buf[0] =  0x2;
        printf("portb: %02i: 0x%02x \n",i,buf[0]);
        f = ftdi_write_data(ftdi2, buf, 1);
        if (f < 0)
            fprintf(stderr,"write failed on channel 2 for 0x%x, error %d (%s)\n", buf[0], f, ftdi_get_error_string(ftdi2));
        usleep(1 * 1000000);
    }
    printf("\n");

    printf("disabling bitbang mode(channel 1)\n");
    ftdi_disable_bitbang(ftdi);
    ftdi_usb_close(ftdi);
    ftdi_free(ftdi);

    printf("disabling bitbang mode(channel 2)\n");
    ftdi_disable_bitbang(ftdi2);
    ftdi_usb_close(ftdi2);
    ftdi_free(ftdi2);

    return 0;
}
