/*  bitbang_cbus.c

    Example to use CBUS bitbang mode of newer chipsets.
    You must enable CBUS bitbang mode in the EEPROM first.

    Thanks to Steve Brown <sbrown@ewol.com> for the
    the information how to do it.

    The top nibble controls input/output and the bottom nibble
    controls the state of the lines set to output. The datasheet isn't clear
    what happens if you set a bit in the output register when that line is
    conditioned for input. This is described in more detail
    in the FT232R bitbang app note.

    BITMASK
    CBUS Bits
    3210 3210
    xxxx xxxx
    |    |------ Output Control 0->LO, 1->HI
    |----------- Input/Output   0->Input, 1->Output

    Example:
    All pins to output with 0 bit high: 0xF1 (11110001)
    Bits 0 and 1 to input, 2 and 3 to output and masked high: 0xCC (11001100)

    The input is standard "0x" hex notation.
    A carriage return terminates the program.

    This program is distributed under the GPL, version 2
*/

#include <stdio.h>
#include <unistd.h>
#include <stdlib.h>
#include <ftdi.h>

int main(void)
{
    struct ftdi_context *ftdi;
    int f;
    unsigned char buf[1];
    unsigned char bitmask;
    char input[10];

    if ((ftdi = ftdi_new()) == 0)
    {
        fprintf(stderr, "ftdi_new failed\n");
        return EXIT_FAILURE;
    }

    f = ftdi_usb_open(ftdi, 0x0403, 0x6001);
    if (f < 0 && f != -5)
    {
        fprintf(stderr, "unable to open ftdi device: %d (%s)\n", f, ftdi_get_error_string(ftdi));
        ftdi_free(ftdi);
        exit(-1);
    }
    printf("ftdi open succeeded: %d\n",f);

    while (1)
    {
        // Set bitmask from input
        fgets(input, sizeof(input) - 1, stdin);
        if (input[0] == '\n') break;
        bitmask = strtol(input, NULL, 0);
        printf("Using bitmask 0x%02x\n", bitmask);
        f = ftdi_set_bitmode(ftdi, bitmask, BITMODE_CBUS);
        if (f < 0)
        {
            fprintf(stderr, "set_bitmode failed for 0x%x, error %d (%s)\n", bitmask, f, ftdi_get_error_string(ftdi));
            ftdi_usb_close(ftdi);
            ftdi_free(ftdi);
            exit(-1);
        }

        // read CBUS
        f = ftdi_read_pins(ftdi, &buf[0]);
        if (f < 0)
        {
            fprintf(stderr, "read_pins failed, error %d (%s)\n", f, ftdi_get_error_string(ftdi));
            ftdi_usb_close(ftdi);
            ftdi_free(ftdi);
            exit(-1);
        }
        printf("Read returned 0x%01x\n", buf[0] & 0x0f);
    }
    printf("disabling bitbang mode\n");
    ftdi_disable_bitbang(ftdi);

    ftdi_usb_close(ftdi);
    ftdi_free(ftdi);

    return 0;
}
