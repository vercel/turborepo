/* purge_test.c
 *
 * Test for purge TX/RX functions.
 *
 * The chip must be wired to loop TX data to RX data (loopback).
 *
 * This program works with "standard" linux drivers and the FTDI1 library.
 *
 * Usage: purge_test [-b baud] [-i interface] [-n msg-size] [-N note] device-specifier
 *   See usage below for more information on command usage.
 *
 * This program works well with the FT4231H which is newer and has large
 * FIFOs. This program does not work well with FT232, either pre or post
 * switching the SIO_RESET_PURGE_TX/SIO_RESET_PURGE_RX values.
 *
 * This needs testing with other devices, which I do not have.
 */
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>
#include <getopt.h>
#include <signal.h>
#include <errno.h>
/* Prevent deprecated messages when building library */
#define _FTDI_DISABLE_DEPRECATED
#include <ftdi.h>

#include <termios.h>		// For baudcodes & linux UARTs
#include <sys/types.h>
#include <sys/stat.h>
#include <fcntl.h>


static struct ftdi_context *ftdi = NULL;
static int dev_fd = -1;
static char * dev_string = NULL;
static int latency_specified = 0;
static int latency = 5;
static int baud = 9600;
static int baud_code = -1;
static enum ftdi_interface interface = INTERFACE_A;
static int msg_size = 80;
static int broken_purge_test = 0;

static const int latency_min = 2;
static const int latency_max = 255;

static volatile long long usec_test_start;



static int ascii2int(const char * str, const char * pgm_name);
static int baud_2_baud_code(int baud);
static long int char_cnt_2_usec(int char_count);
static long int drain();
static int flush(int queue_selector);
static long long int get_time_usec();

static const int flushQueueSelector[] = {
    TCIFLUSH, TCOFLUSH, TCIOFLUSH }; /* See /usr/include/bits/termios.h */
static const char * flushTestName[] = {
  "Input-only", "Output-only", "Input+Output" };
static const char * expected[] = {
    "last portion of message",
    "first portion of message",
    "mid-message characters",
};


static const char * chip_types[] = {
    "am",
    "bm",
    "2232C",
    "R",
    "2232H",
    "4232H",
    "232H",
    "230X",
};

#ifndef ARRAY_SIZE
#  define ARRAY_SIZE(x) (sizeof(x)/sizeof(x[0]))
#endif



/**********************************************************************
 */
static void
usage(const char *argv0)
{
   fprintf(stderr,
           "Usage: %s [options...] device-specifier\n"
           "Flush test for UARTS.\n"
	   " with loopback connector\n"
           "    [-b baud]        baud rate (e.g., 300, 600, 1200, ...230400)\n"
           "    [-i {a|b|c|d}]   FTDI interface for chips which have multiple UARTS\n"
	   "    [-l latency]     Latency (%d..%d)\n"
           "    [-n msg-size]    Number of bytes in test message\n"
           "    [-N note]        Note for the output\n"
	   "    [-P]             Use broken libftdi1 purge methods (over new flush)\n"
           "\n"
           "    device-specifier String specifying the UART.  If the first character\n"
	   "                     is the '/' character, the program assumes a Linux UART\n"
	   "                     is to be tested and the string would be something like\n"
	   "                     '/dev/ttyS0' or '/dev/ttyUSB0'. Otherwise, the program\n"
	   "                     assumes an FTDI device is being tested with the FTDI1\n"
	   "                     library. The device-specifier must be a string\n"
	   "                     accepted by the ftdi_usb_open_string function. An\n"
	   "                     example would be 'i:0x0403:0x6011[:index]'.\n"
	   "\n"
	   "NOTE: To function correctly, this program requires a loopback connector\n"
	   "      attached to the UART under test.\n"
           "\n"
           "Adapted from stream_test.c 2018. Eric Schott <els6@psu.edu>\n"
           "Copyright (C) 2009 Micah Dowty <micah@navi.cx>\n"
           "Adapted for use with libftdi (C) 2010 Uwe Bonnes <bon@elektron.ikp.physik.tu-darmstadt.de>\n",
           argv0, latency_min, latency_max);
   exit(1);
}


/**********************************************************************
 */
int main(int argc, char **argv)
{
    int c, i;
    int option_index;
    int test;
    unsigned char * msg;
    unsigned char * retMsg;
    char * note = NULL;
    char * note_default = NULL;
    size_t retMsgSize;
    long int msg_xmit_time_us;
    static struct option long_options[] = {{NULL},};

    while ((c = getopt_long(argc, argv, "n:b:i:l:N:P", long_options, &option_index)) !=- 1)
        switch (c)
        {
        case -1:
            break;
        case 'b':
            baud = ascii2int(optarg, argv[0]);
            break;
        case 'i':
            if (optarg == NULL || strlen(optarg) != 1)
                usage(argv[0]);
            switch (optarg[0])
            {
            case 'a':
            case 'A':
                interface = INTERFACE_A;
                break;

            case 'b':
            case 'B':
                interface = INTERFACE_B;
                break;

            case 'c':
            case 'C':
                interface = INTERFACE_C;
                break;

            case 'd':
            case 'D':
                interface = INTERFACE_D;
                break;

            default:
                usage(argv[0]);
            }
            break;
        case 'l':
            latency = ascii2int(optarg, argv[0]);
            if (latency < latency_min || latency > latency_max)
            {
	      fprintf(stderr, "latency [-l] must be an integer in the range %d..%d\n",
			latency_min, latency_max);
                usage(argv[0]);
            }
	    latency_specified = 1;
            break;
        case 'n':
            msg_size = ascii2int(optarg, argv[0]);
            if (msg_size < 1)
            {
                fprintf(stderr, "msg-size [-n] must be an integer greater than 0\n");
                usage(argv[0]);
            }
            break;
        case 'N':
            note = optarg;
            break;
	case 'P':
	    broken_purge_test = 1;
	    break;
        default:
            usage(argv[0]);
        }

    if (optind == argc)
        usage(argv[0]);

    if (optind == argc - 1)
    {
        // Exactly one extra argument- a dump file
        dev_string = argv[optind];
    }
    else if (optind < argc)
    {
        // Too many extra args
        usage(argv[0]);
    }

    baud_code = baud_2_baud_code(baud);
    if (baud_code < 1)
    {
        fprintf(stderr, "Invalid baud [-b]\n");
        usage(argv[0]);
    }

    if (dev_string[0] == '/')
    {
        struct termios termios;

	if (latency_specified) {
	  fprintf(stderr, "Latency (-l) option not support on this device; ignored\n");
	}

	if (broken_purge_test) {
	  fprintf(stderr, "Broken-purge (-P) option not support with Linux kernel driver\n");
	  return EXIT_FAILURE;
	}

        dev_fd = open(dev_string, O_NOCTTY | O_RDWR);
        if (dev_fd < 0)
        {
            fprintf(stderr, "Error opening Linux device \"%s\": %s\n",
                    dev_string, strerror(errno));
            return EXIT_FAILURE;
        }

        if (! isatty(dev_fd))
        {
            fprintf(stderr, "Not a TTY device: \"%s\"\n", dev_string);
            return EXIT_FAILURE;
        }

        if (tcgetattr(dev_fd, &termios) == -1)
        {
            fprintf(stderr, "Error getting TTY attributes for \"%s\": %s\n",
                    dev_string, strerror(errno));
            return EXIT_FAILURE;
        }

	note_default = "Linux kernel driver";

        cfmakeraw(&termios);

        termios.c_cflag &=
            ~(CSTOPB | CRTSCTS);

        termios.c_cflag &= ~CSIZE;
        termios.c_cflag |= CS8;

        cfsetspeed(&termios, baud_code);

        termios.c_cflag |=
            CLOCAL;

        termios.c_cc[VMIN] = 1;	// Character at a time input
        termios.c_cc[VTIME] = 0;	// with blocking

        if (tcsetattr(dev_fd, TCSAFLUSH, &termios) == -1) {
            fprintf(stderr, "Error setting TTY attributes for \"%s\": %s\n", 
                    dev_string, strerror(errno));
            return EXIT_FAILURE;
        }
    }
    else
    {

        if ((ftdi = ftdi_new()) == 0)
        {
            fprintf(stderr, "ftdi_new failed\n");
            return EXIT_FAILURE;
        }

        if (ftdi_set_interface(ftdi, interface) < 0)
        {
            fprintf(stderr, "ftdi_set_interface failed\n");
            ftdi_free(ftdi);
            return EXIT_FAILURE;
        }

        if (ftdi_usb_open_string(ftdi, dev_string) < 0)
        {
            fprintf(stderr,"Error opening ftdi device \"%s\": %s\n", dev_string,
                    ftdi_get_error_string(ftdi));
            ftdi_free(ftdi);
            return EXIT_FAILURE;
        }

	if(ftdi_set_latency_timer(ftdi, (unsigned char) latency))
	{
	    if (latency_specified &&
                (ftdi->type == TYPE_AM || ftdi->type == TYPE_232H)) {
	        fprintf(stderr, "Latency (-l) option not support on this device; ignored\n");
	    } else if (ftdi->type != TYPE_AM && ftdi->type != TYPE_232H) {
                fprintf(stderr,"Error setting latency for ftdi device \"%s\" (%d): %s\n",
                        dev_string, ftdi->type, ftdi_get_error_string(ftdi));
                ftdi_free(ftdi);
                return EXIT_FAILURE;
            }
	}

        if (ftdi_set_line_property2(ftdi, BITS_8, STOP_BIT_1, NONE, BREAK_OFF) < 0)
        {
            fprintf(stderr,"Error setting line properties ftdi device \"%s\": %s\n", dev_string,
                    ftdi_get_error_string(ftdi));
            ftdi_free(ftdi);
            return EXIT_FAILURE;
        }

        if (ftdi_set_baudrate(ftdi, baud) < 0)
        {
            fprintf(stderr,"Error setting baud rate for ftdi device \"%s\": %s\n", dev_string,
                    ftdi_get_error_string(ftdi));
            ftdi_free(ftdi);
            return EXIT_FAILURE;
        }

        if (ftdi_setflowctrl(ftdi, SIO_DISABLE_FLOW_CTRL))
        {
            fprintf(stderr,"Error setting flow control for ftdi device \"%s\": %s\n", dev_string,
                    ftdi_get_error_string(ftdi));
            ftdi_free(ftdi);
            return EXIT_FAILURE;
        }

	if (broken_purge_test)
	    note_default = "libftdi w/ deprecated purge";
	else
	    note_default = "libftdi w/ new flush methods";
    }

    printf("Purge (tcflush) test for device %s\n", dev_string);
    printf("Note: %s\n", (note ? note : note_default));

    if (dev_fd < 0)
    {
        if (ftdi->type >0 && ftdi->type < ARRAY_SIZE(chip_types))
            printf("FTDI chip type is %d (%s)\n",
                   ftdi->type, chip_types[ftdi->type]);
        else
            printf("FTDI chip type is %d (unknown)\n", ftdi->type);
    }

    printf("# purge_test" );
    for (c = 1; c < argc; ++c)
    {
        const char *p = argv[c];
        while (*p != '\0')
        {
            if (*p == ' ')
                break;
            ++p;
        }
        if (*p == ' ')
            printf(" '%s'", argv[c]);
        else
            printf(" %s", argv[c]);
    }
    printf("\n");

    msg_xmit_time_us = char_cnt_2_usec(msg_size);
    printf("%d chars at %d baud takes about %.0f ms to transmit\n", msg_size,
           baud, msg_xmit_time_us * .001);

    msg = malloc(msg_size + 1);
    if (msg == NULL)
    {
        fprintf(stderr, "Could not allocate send message buffer\n");
        return EXIT_FAILURE;
    }

    {
        char dataChar = '0' + ((get_time_usec() / 1000) % 31);
        char next = 'A';
        for (i = 0; i < msg_size; ++i) {
            if (dataChar == '`')
            {
                msg[i] = next++;
                ++dataChar;
            }
            else
                msg[i] = dataChar++;

            if (dataChar > 'z') {
                dataChar = '`';
            }
        }
        msg[msg_size] = '\0';
    }

    printf("TX Message is \"%s\"\n", msg);

    retMsgSize = 2 * msg_size;
    retMsg = malloc(retMsgSize);
    if (retMsg == NULL)
    {
        fprintf(stderr, "Could not allocate received message buffer\n");
        return EXIT_FAILURE;
    }

    flush(TCIOFLUSH);

    for (test = 0; test <= 2; ++test)
    {
        long long usec_delay;
        long long usec_to_now;
        int rc;

        printf("\n********  Test purge %s; expect %s  ********\n"
	       "  --              Flushing UART\n",
               flushTestName[test], expected[test]);
        flush(TCIOFLUSH);
        usleep(msg_xmit_time_us);
        flush(TCIOFLUSH);
        usleep(100000);

        usec_test_start = get_time_usec();
        if (dev_fd >= 0)
            rc = write(dev_fd, msg, msg_size);
        else
            rc = ftdi_write_data(ftdi, msg, msg_size);

        if (rc != msg_size)
        {
            fprintf(stderr, "Data write was short: %d: %s\n",
                    rc, ftdi_get_error_string(ftdi));
            exit(1);
        }
        usec_to_now = get_time_usec() - usec_test_start;
        usec_delay = msg_xmit_time_us / 2 - usec_to_now;
        if (usec_delay < 0)
            usec_delay = 0;
        printf("  -- %9.1f ms Write completes; delaying to TX midpoint (%.1f ms)\n", 
               usec_to_now * .001, usec_delay * .001);
        if (usec_delay > 0)
            usleep(usec_delay);

        printf("  -- %9.1f ms Issuing %s flush (purge)\n", 
               (get_time_usec() - usec_test_start) * .001,
               flushTestName[test]);
        flush(flushQueueSelector[test]);

        printf("  -- %9.1f ms Calling drain to wait for transmit to complete\n", 
               (get_time_usec() - usec_test_start) * .001);
        drain();

        usec_to_now = get_time_usec() - usec_test_start;

        /* If input only flush, check drain time. */
        if (flushQueueSelector[test] == TCIFLUSH &&
            usec_to_now < (msg_xmit_time_us * 90ll) / 100ll)
        {
            usec_delay = (msg_xmit_time_us * 110ll) / 100ll - usec_to_now;
            printf("  -- %9.1f ms Drain() completed too early; expected at least %.1f ms\n"
                   "                  Delaying for %.1f ms\n", 
                   usec_to_now * .001,
                   ((msg_xmit_time_us * 90ll) / 100ll) * .001,
                   usec_delay * .001);
            usleep(usec_delay);
        }
        else
	{
            printf("  -- %9.1f ms Drain() reports completed; timing OK; delaying for 4 bytes\n", 
                   (get_time_usec() - usec_test_start) * .001);
            usleep(char_cnt_2_usec(4));
        }

        printf("  -- %9.1f ms Reading data.\n",
               (get_time_usec() - usec_test_start) * .001);
        if (dev_fd >= 0)
            rc = read(dev_fd, retMsg, retMsgSize);
        else
            rc = ftdi_read_data(ftdi, retMsg, retMsgSize - 1);

        usec_to_now = get_time_usec() - usec_test_start;
        if (rc < 0)
        {
            fprintf(stderr, "  -- %9.1f ms Read returned error %s\n",
                    usec_to_now * .001,
                    (dev_fd >= 0 ? strerror(errno) : ftdi_get_error_string(ftdi)));
            exit(1);
        }
        retMsg[rc] = '\0';
        printf("  -- %9.1f ms Read returns %d bytes; msg: \"%s\"\n",
               usec_to_now * .001, rc, retMsg);

        usleep(char_cnt_2_usec(10));

    }


    if (dev_fd >= 0)
    {
        close(dev_fd);
    }
    else
    {
        ftdi_usb_close(ftdi);
        ftdi_free(ftdi);
    }

    exit (0);
}

/**********************************************************************
 */
static int ascii2int(const char * str, const char * pgm_name)
{
    int rc;
    char * endptr;
    if (str == NULL || strlen(str) == 0)
        usage(pgm_name);
    rc = strtol(str, &endptr, 10);
    if (endptr == str || *endptr != '\0')
        usage(pgm_name);
    return rc;
}


/**********************************************************************
 */
static struct Baud_Table {
	int32_t baud, baud_code;
} baud_table [] =
{
	{ 50,     B50     },
	{ 75,     B75     },
	{ 110,    B110    },
	{ 134,    B134    },
	{ 150,    B150    },
	{ 200,    B200    },
	{ 300,    B300    },
	{ 600,    B600    },
	{ 1200,   B1200   },
	{ 1800,   B1800   },
	{ 2400,   B2400   },
	{ 4800,   B4800   },
	{ 9600,   B9600   },
	{ 19200,  B19200  },
	{ 38400,  B38400  },
	{ 57600,  B57600  },
	{ 115200, B115200 },
	{ 230400, B230400 },
	{ -1,     -1,     }
};

/**********************************************************************
 */
static int baud_2_baud_code(int baud)
{
    struct Baud_Table *p;

    for (p = baud_table ; p->baud != -1; ++p) {
        if (p->baud == baud)
            break;
    }
    return p->baud_code;
}


static long int char_cnt_2_usec(int char_count)
{
    long long bits = 8 + 1 + 1;                 /* Number of bits in each character */
    bits *= (char_count == 0 ? 1 : char_count); /* Total number of bits */
    bits *= 1000000;                            /* Convert to us */
    lldiv_t parts = lldiv(bits, baud);          /* Number of us for message */
    return (parts.quot + 1);
}


static long int drain()
{
    long int rc = 0;
    long long start_time = get_time_usec();
    if (dev_fd >= 0)
        rc = tcdrain(dev_fd);
    else
    {
        long int sleep_interval = char_cnt_2_usec(10);
        while (1) {
            unsigned short modem_status = 0;
            int rc = ftdi_poll_modem_status(ftdi, &modem_status);
            if (rc < 0)
                return -1;
            if (modem_status & (1 << (6 + 8))) {
                break;
            }
            usleep(sleep_interval);
        }
    }
    if (rc < 0)
        return rc;
    usleep(char_cnt_2_usec(2));
    return get_time_usec() - start_time;
}


static int flush(int queue_selector)
{
    int rc;
    if (dev_fd >= 0)
        rc = tcflush(dev_fd, queue_selector);
    else if (! broken_purge_test)
    {
        switch (queue_selector) {

        case TCIOFLUSH:
            rc = ftdi_tcioflush(ftdi);
            break;

        case TCIFLUSH:
            rc = ftdi_tciflush(ftdi);
            break;

        case TCOFLUSH:
            rc = ftdi_tcoflush(ftdi);
            break;

        default:
            errno = EINVAL;
            return -1;
        }
    }
    else
    {
        switch (queue_selector) {

        case TCIOFLUSH:
            rc = ftdi_usb_purge_buffers(ftdi);
            break;

        case TCIFLUSH:
            rc = ftdi_usb_purge_rx_buffer(ftdi);
            break;

        case TCOFLUSH:
            rc = ftdi_usb_purge_tx_buffer(ftdi);
            break;

        default:
            errno = EINVAL;
            return -1;
        }
    }

    return rc;
}


static long long int get_time_usec()
{
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return tv.tv_sec * 1000000ll + tv.tv_usec;
}
