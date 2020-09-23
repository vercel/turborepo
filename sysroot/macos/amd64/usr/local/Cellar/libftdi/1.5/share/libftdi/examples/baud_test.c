/* baud_test.c
 *
 * test setting the baudrate and compare it with the expected runtime
 *
 * options:
 *  -p <devicestring> defaults to "i:0x0403:0x6001" (this is the first FT232R with default id)
 *       d:<devicenode> path of bus and device-node (e.g. "003/001") within usb device tree (usually at /proc/bus/usb/)
 *       i:<vendor>:<product> first device with given vendor and product id,
 *                            ids can be decimal, octal (preceded by "0") or hex (preceded by "0x")
 *       i:<vendor>:<product>:<index> as above with index being the number of the device (starting with 0)
 *                            if there are more than one
 *       s:<vendor>:<product>:<serial> first device with given vendor id, product id and serial string
 *  -d <datasize to send in bytes>
 *  -b <baudrate> (divides by 16 if bitbang as taken from the ftdi datasheets)
 *  -m <mode to use> r: serial a: async bitbang s:sync bitbang
 *  -c <chunksize>
 *
 * (C) 2009 by Gerd v. Egidy <gerd.von.egidy@intra2net.com>
 *
 * This program is free software; you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation; either version 2 of the License, or
 * (at your option) any later version.

 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 */

#include <sys/time.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <ftdi.h>

double get_prec_time()
{
    struct timeval tv;
    double res;

    gettimeofday(&tv,NULL);

    res=tv.tv_sec;
    res+=((double)tv.tv_usec/1000000);

    return res;
}

int main(int argc, char **argv)
{
    struct ftdi_context *ftdi;
    int i, t;
    unsigned char *txbuf;
    unsigned char *rxbuf;
    double start, duration, plan;
    int retval= 0;

    // default values
    int baud=9600;
    int set_baud;
    int datasize=100000;

    char default_devicedesc[] = "i:0x0403:0x6001";
    char *devicedesc=default_devicedesc;
    int txchunksize=256;
    enum ftdi_mpsse_mode test_mode=BITMODE_BITBANG;

    while ((t = getopt (argc, argv, "b:d:p:m:c:")) != -1)
    {
        switch (t)
        {
            case 'd':
                datasize = atoi (optarg);
                break;
            case 'm':
                switch (*optarg)
                {
                    case 'r':
                        // serial
                        test_mode=BITMODE_RESET;
                        break;
                    case 'a':
                        // async
                        test_mode=BITMODE_BITBANG;
                        break;
                    case 's':
                        // sync
                        test_mode=BITMODE_SYNCBB;
                        break;
                }
                break;
            case 'b':
                baud = atoi (optarg);
                break;
            case 'p':
                devicedesc=optarg;
                break;
            case 'c':
                txchunksize = atoi (optarg);
                break;
        }
    }

    txbuf=malloc(txchunksize);
    rxbuf=malloc(txchunksize);
    if (txbuf == NULL || rxbuf == NULL)
    {
        fprintf(stderr, "can't malloc\n");
        return EXIT_FAILURE;
    }

    if ((ftdi = ftdi_new()) == 0)
    {
        fprintf(stderr, "ftdi_new failed\n");
        retval = EXIT_FAILURE;
        goto done;
    }

    if (ftdi_usb_open_string(ftdi, devicedesc) < 0)
    {
        fprintf(stderr,"Can't open ftdi device: %s\n",ftdi_get_error_string(ftdi));
        retval = EXIT_FAILURE;
        goto do_deinit;
    }

    set_baud=baud;
    if (test_mode!=BITMODE_RESET)
    {
        // we do bitbang, so real baudrate / 16
        set_baud=baud/16;
    }

    ftdi_set_baudrate(ftdi,set_baud);
    printf("real baudrate used: %d\n",(test_mode==BITMODE_RESET) ? ftdi->baudrate : ftdi->baudrate*16);

    if (ftdi_set_bitmode(ftdi, 0xFF,test_mode) < 0)
    {
        fprintf(stderr,"Can't set mode: %s\n",ftdi_get_error_string(ftdi));
        retval = EXIT_FAILURE;
        goto do_close;
    }

    if (test_mode==BITMODE_RESET)
    {
        // serial 8N1: 8 data bits, 1 startbit, 1 stopbit
        plan=((double)(datasize*10))/baud;
    }
    else
    {
        // bitbang means 8 bits at once
        plan=((double)datasize)/baud;
    }

    printf("this test should take %.2f seconds\n",plan);

    // prepare data to send: 0 and 1 bits alternating (except for serial start/stopbit):
    // maybe someone wants to look at this with a scope or logic analyzer
    for (i=0; i<txchunksize; i++)
    {
        if (test_mode==BITMODE_RESET)
            txbuf[i]=0xAA;
        else
            txbuf[i]=(i%2) ? 0xff : 0;
    }

    if (ftdi_write_data_set_chunksize(ftdi, txchunksize) < 0 ||
            ftdi_read_data_set_chunksize(ftdi, txchunksize) < 0)
    {
        fprintf(stderr,"Can't set chunksize: %s\n",ftdi_get_error_string(ftdi));
        retval = EXIT_FAILURE;
        goto do_close;
    }

    if (test_mode==BITMODE_SYNCBB)
    {
        // completely clear the receive buffer before beginning
        while (ftdi_read_data(ftdi, rxbuf, txchunksize)>0);
    }

    start=get_prec_time();

    // don't wait for more data to arrive, take what we get and keep on sending
    // yes, we really would like to have libusb 1.0+ with async read/write...
    ftdi->usb_read_timeout=1;

    i=0;
    while (i < datasize)
    {
        int sendsize=txchunksize;
        if (i+sendsize > datasize)
            sendsize=datasize-i;

        if ((sendsize=ftdi_write_data(ftdi, txbuf, sendsize)) < 0)
        {
            fprintf(stderr,"write failed at %d: %s\n",
                    i, ftdi_get_error_string(ftdi));
            retval = EXIT_FAILURE;
            goto do_close;
        }

        i+=sendsize;

        if (test_mode==BITMODE_SYNCBB)
        {
            // read the same amount of data as sent
            ftdi_read_data(ftdi, rxbuf, sendsize);
        }
    }

    duration=get_prec_time()-start;
    printf("and took %.4f seconds, this is %.0f baud or factor %.3f\n",duration,(plan*baud)/duration,plan/duration);
do_close:
    ftdi_usb_close(ftdi);
do_deinit:
    ftdi_free(ftdi);
done:
    if(rxbuf)
        free(rxbuf);
    if(txbuf)
        free(txbuf);
    exit (retval);
}
