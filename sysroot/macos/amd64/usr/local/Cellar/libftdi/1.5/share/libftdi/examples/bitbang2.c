/* ftdi_out.c
 *
 * Output a (stream of) byte(s) in bitbang mode to the
 * ftdi245 chip that is (hopefully) attached.
 *
 * We have a little board that has a FT245BM chip and
 * the 8 outputs are connected to several different
 * things that we can turn on and off with this program.
 *
 * If you have an idea about hardware that can easily
 * interface onto an FTDI chip, I'd like to collect
 * ideas. If I find it worthwhile to make, I'll consider
 * making it, I'll even send you a prototype (against
 * cost-of-material) if you want.
 *
 * At "harddisk-recovery.nl" they have a little board that
 * controls the power to two harddrives and two fans.
 *
 * -- REW R.E.Wolff@BitWizard.nl
 *
 *
 *
 * This program was based on libftdi_example_bitbang2232.c
 * which doesn't carry an author or attribution header.
 *
 *
 * This program is distributed under the GPL, version 2.
 * Millions copies of the GPL float around the internet.
 */


#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <ftdi.h>

void ftdi_fatal (struct ftdi_context *ftdi, char *str)
{
    fprintf (stderr, "%s: %s\n",
             str, ftdi_get_error_string (ftdi));
    ftdi_free(ftdi);
    exit (1);
}

int main(int argc, char **argv)
{
    struct ftdi_context *ftdi;
    int i, t;
    unsigned char data;
    int delay = 100000; /* 100 thousand microseconds: 1 tenth of a second */

    while ((t = getopt (argc, argv, "d:")) != -1)
    {
        switch (t)
        {
            case 'd':
                delay = atoi (optarg);
                break;
        }
    }

    if ((ftdi = ftdi_new()) == 0)
    {
        fprintf(stderr, "ftdi_bew failed\n");
        return EXIT_FAILURE;
    }

    if (ftdi_usb_open(ftdi, 0x0403, 0x6001) < 0)
        ftdi_fatal (ftdi, "Can't open ftdi device");

    if (ftdi_set_bitmode(ftdi, 0xFF, BITMODE_BITBANG) < 0)
        ftdi_fatal (ftdi, "Can't enable bitbang");

    for (i=optind; i < argc ; i++)
    {
        sscanf (argv[i], "%x", &t);
        data = t;
        if (ftdi_write_data(ftdi, &data, 1) < 0)
        {
            fprintf(stderr,"write failed for 0x%x: %s\n",
                    data, ftdi_get_error_string(ftdi));
        }
        usleep(delay);
    }

    ftdi_usb_close(ftdi);
    ftdi_free(ftdi);
    exit (0);
}
