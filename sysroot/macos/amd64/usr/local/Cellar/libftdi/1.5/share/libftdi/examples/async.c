/* Libftdi example for asynchronous read/write.

   This program is distributed under the GPL, version 2
*/

/* This program switches to MPSSE mode, and sets and then reads back
 * the high byte 3 times with three different values.
 * The expected read values are hard coded in ftdi_init
 * with 0x00, 0x55 and 0xaa
 *
 * Make sure that that nothing else drives some bit of the high byte
 * or expect a collision for a very short time and some differences
 * in the data read back.
 *
 * Result should be the same without any option or with either
 * -r or -w or -b.
 */


#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
#include <unistd.h>
#include <getopt.h>
#include <ftdi.h>

int main(int argc, char **argv)
{
    struct ftdi_context *ftdi;
    int do_read = 0;
    int do_write = 0;
    int i, f, retval;

    if ((ftdi = ftdi_new()) == 0)
    {
        fprintf(stderr, "Failed to allocate ftdi structure :%s \n",
                ftdi_get_error_string(ftdi));
        return EXIT_FAILURE;
    }

    while ((i = getopt(argc, argv, "brw")) != -1)
    {
        switch (i)
        {
            case 'b':
                do_read = 1;
                do_write = 1;
                break;
            case 'r':
                do_read = 1;
                break;
            case 'w':
                do_write  = 1;
                break;
            default:
                fprintf(stderr, "usage: %s [options]\n", *argv);
                fprintf(stderr, "\t-b do synchronous read and write\n");
                fprintf(stderr, "\t-r do synchronous read\n");
                fprintf(stderr, "\t-w do synchronous write\n");
                retval = -1;
                goto done;
        }
    }

    /* Select first free interface */
    ftdi_set_interface(ftdi, INTERFACE_ANY);

    struct ftdi_device_list *devlist;
    int res;
    if ((res = ftdi_usb_find_all(ftdi, &devlist, 0, 0)) < 0)
    {
        fprintf(stderr, "No FTDI with default VID/PID found\n");
        retval =  EXIT_FAILURE;
        goto do_deinit;
    }
    if (res > 0)
    {
        int i = 1;
        f = ftdi_usb_open_dev(ftdi, devlist[0].dev);
        if (f < 0)
        {
            fprintf(stderr, "Unable to open device %d: (%s)",
                    i, ftdi_get_error_string(ftdi));
            retval = -1;
            goto do_deinit;
        }
    }
    else
    {
        fprintf(stderr, "No devices found\n");
        retval = -1;
        goto do_deinit;
    }
    ftdi_list_free(&devlist);
    int err = ftdi_tcioflush(ftdi);
    if (err != 0) {
        fprintf(stderr, "ftdi_tcioflush: %d: %s\n",
                err, ftdi_get_error_string(ftdi));
        retval = -1;
        goto do_deinit;
    }
    /* Reset MPSSE controller. */
    err = ftdi_set_bitmode(ftdi, 0,  BITMODE_RESET);
    if (err != 0) {
        fprintf(stderr, "ftdi_set_bitmode: %d: %s\n",
                err, ftdi_get_error_string(ftdi));
        retval = -1;
        goto do_deinit;
   }
    /* Enable MPSSE controller. Pin directions are set later.*/
    err = ftdi_set_bitmode(ftdi, 0, BITMODE_MPSSE);
    if (err != 0) {
        fprintf(stderr, "ftdi_set_bitmode: %d: %s\n",
                err, ftdi_get_error_string(ftdi));
        return -1;
    }
#define DATA_TO_READ 3
    uint8_t ftdi_init[] = {TCK_DIVISOR, 0x00, 0x00,
                             /* Set High byte to zero.*/
                             SET_BITS_HIGH, 0, 0xff,
                             GET_BITS_HIGH,
                             /* Set High byte to 0x55.*/
                             SET_BITS_HIGH, 0x55, 0xff,
                             GET_BITS_HIGH,
                             /* Set High byte to 0xaa.*/
                             SET_BITS_HIGH, 0xaa, 0xff,
                             GET_BITS_HIGH,
                             /* Set back to high impedance.*/
                             SET_BITS_HIGH, 0x00, 0x00 };
    struct ftdi_transfer_control *tc_read;
    struct ftdi_transfer_control *tc_write;
    uint8_t data[3];
    if (do_read) {
        tc_read = ftdi_read_data_submit(ftdi, data, DATA_TO_READ);
    }
    if (do_write) {
        tc_write = ftdi_write_data_submit(ftdi, ftdi_init, sizeof(ftdi_init));
        int transfer = ftdi_transfer_data_done(tc_write);
        if (transfer != sizeof(ftdi_init)) {
            printf("Async write failed : %d\n", transfer);
        }
    } else {
        int written = ftdi_write_data(ftdi, ftdi_init, sizeof(ftdi_init));
        if (written != sizeof(ftdi_init)) {
            printf("Sync write failed: %d\n", written);
        }
    }
    if (do_read) {
        int transfer = ftdi_transfer_data_done(tc_read);
        if (transfer != DATA_TO_READ) {
            printf("Async Read failed:%d\n", transfer);
        }
    } else {
        int index = 0;
        ftdi->usb_read_timeout = 1;
        int i = 1000; /* Fail if read did not succeed in 1 second.*/
        while (i--) {
            int res = ftdi_read_data(ftdi, data + index, 3 - index);
            if (res < 0) {
                printf("Async read failure at %d\n", index);
            } else {
                index += res;
            }
            if (res == 3) {
                break;
            }
        }
        if (i < 1) {
            printf("Async read unsuccessful\n");
        }
    }
    printf("Read %02x %02x %02x\n", data[0], data[1], data[2]);
done:
    ftdi_usb_close(ftdi);
do_deinit:
    ftdi_free(ftdi);
    return retval;
}
