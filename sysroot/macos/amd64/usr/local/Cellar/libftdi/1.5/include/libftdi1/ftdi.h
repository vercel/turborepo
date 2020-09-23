/***************************************************************************
                          ftdi.h  -  description
                             -------------------
    begin                : Fri Apr 4 2003
    copyright            : (C) 2003-2020 by Intra2net AG and the libftdi developers
    email                : opensource@intra2net.com
    SPDX-License-Identifier: LGPL-2.1-only
 ***************************************************************************/

/***************************************************************************
 *                                                                         *
 *   This program is free software; you can redistribute it and/or modify  *
 *   it under the terms of the GNU Lesser General Public License           *
 *   version 2.1 as published by the Free Software Foundation;             *
 *                                                                         *
 ***************************************************************************/

#ifndef __libftdi_h__
#define __libftdi_h__

#include <stdint.h>
#ifndef _WIN32
#include <sys/time.h>
#endif

/* Define _FTDI_DISABLE_DEPRECATED to disable deprecated messages. */
#ifdef _FTDI_DISABLE_DEPRECATED
#define _Ftdi_Pragma(_msg)
#else
#define _Ftdi_Pragma(_msg) _Pragma(_msg)
#endif

/* 'interface' might be defined as a macro on Windows, so we need to
 * undefine it so as not to break the current libftdi API, because
 * struct ftdi_context has an 'interface' member
 * As this can be problematic if you include windows.h after ftdi.h
 * in your sources, we force windows.h to be included first. */
#if defined(_WIN32) || defined(__CYGWIN__) || defined(_WIN32_WCE)
#include <windows.h>
#if defined(interface)
#undef interface
#endif
#endif

/** FTDI chip type */
enum ftdi_chip_type
{
    TYPE_AM=0,
    TYPE_BM=1,
    TYPE_2232C=2,
    TYPE_R=3,
    TYPE_2232H=4,
    TYPE_4232H=5,
    TYPE_232H=6,
    TYPE_230X=7,
};
/** Parity mode for ftdi_set_line_property() */
enum ftdi_parity_type { NONE=0, ODD=1, EVEN=2, MARK=3, SPACE=4 };
/** Number of stop bits for ftdi_set_line_property() */
enum ftdi_stopbits_type { STOP_BIT_1=0, STOP_BIT_15=1, STOP_BIT_2=2 };
/** Number of bits for ftdi_set_line_property() */
enum ftdi_bits_type { BITS_7=7, BITS_8=8 };
/** Break type for ftdi_set_line_property2() */
enum ftdi_break_type { BREAK_OFF=0, BREAK_ON=1 };

/** MPSSE bitbang modes */
enum ftdi_mpsse_mode
{
    BITMODE_RESET  = 0x00,    /**< switch off bitbang mode, back to regular serial/FIFO */
    BITMODE_BITBANG= 0x01,    /**< classical asynchronous bitbang mode, introduced with B-type chips */
    BITMODE_MPSSE  = 0x02,    /**< MPSSE mode, available on 2232x chips */
    BITMODE_SYNCBB = 0x04,    /**< synchronous bitbang mode, available on 2232x and R-type chips  */
    BITMODE_MCU    = 0x08,    /**< MCU Host Bus Emulation mode, available on 2232x chips */
    /* CPU-style fifo mode gets set via EEPROM */
    BITMODE_OPTO   = 0x10,    /**< Fast Opto-Isolated Serial Interface Mode, available on 2232x chips  */
    BITMODE_CBUS   = 0x20,    /**< Bitbang on CBUS pins of R-type chips, configure in EEPROM before */
    BITMODE_SYNCFF = 0x40,    /**< Single Channel Synchronous FIFO mode, available on 2232H chips */
    BITMODE_FT1284 = 0x80,    /**< FT1284 mode, available on 232H chips */
};

/** Port interface for chips with multiple interfaces */
enum ftdi_interface
{
    INTERFACE_ANY = 0,
    INTERFACE_A   = 1,
    INTERFACE_B   = 2,
    INTERFACE_C   = 3,
    INTERFACE_D   = 4
};

/** Automatic loading / unloading of kernel modules */
enum ftdi_module_detach_mode
{
    AUTO_DETACH_SIO_MODULE = 0,
    DONT_DETACH_SIO_MODULE = 1,
    AUTO_DETACH_REATACH_SIO_MODULE = 2
};

/* Shifting commands IN MPSSE Mode*/
#define MPSSE_WRITE_NEG 0x01   /* Write TDI/DO on negative TCK/SK edge*/
#define MPSSE_BITMODE   0x02   /* Write bits, not bytes */
#define MPSSE_READ_NEG  0x04   /* Sample TDO/DI on negative TCK/SK edge */
#define MPSSE_LSB       0x08   /* LSB first */
#define MPSSE_DO_WRITE  0x10   /* Write TDI/DO */
#define MPSSE_DO_READ   0x20   /* Read TDO/DI */
#define MPSSE_WRITE_TMS 0x40   /* Write TMS/CS */

/* FTDI MPSSE commands */
#define SET_BITS_LOW   0x80
/*BYTE DATA*/
/*BYTE Direction*/
#define SET_BITS_HIGH  0x82
/*BYTE DATA*/
/*BYTE Direction*/
#define GET_BITS_LOW   0x81
#define GET_BITS_HIGH  0x83
#define LOOPBACK_START 0x84
#define LOOPBACK_END   0x85
#define TCK_DIVISOR    0x86
/* H Type specific commands */
#define DIS_DIV_5       0x8a
#define EN_DIV_5        0x8b
#define EN_3_PHASE      0x8c
#define DIS_3_PHASE     0x8d
#define CLK_BITS        0x8e
#define CLK_BYTES       0x8f
#define CLK_WAIT_HIGH   0x94
#define CLK_WAIT_LOW    0x95
#define EN_ADAPTIVE     0x96
#define DIS_ADAPTIVE    0x97
#define CLK_BYTES_OR_HIGH 0x9c
#define CLK_BYTES_OR_LOW  0x9d
/*FT232H specific commands */
#define DRIVE_OPEN_COLLECTOR 0x9e
/* Value Low */
/* Value HIGH */ /*rate is 12000000/((1+value)*2) */
#define DIV_VALUE(rate) (rate > 6000000)?0:((6000000/rate -1) > 0xffff)? 0xffff: (6000000/rate -1)

/* Commands in MPSSE and Host Emulation Mode */
#define SEND_IMMEDIATE 0x87
#define WAIT_ON_HIGH   0x88
#define WAIT_ON_LOW    0x89

/* Commands in Host Emulation Mode */
#define READ_SHORT     0x90
/* Address_Low */
#define READ_EXTENDED  0x91
/* Address High */
/* Address Low  */
#define WRITE_SHORT    0x92
/* Address_Low */
#define WRITE_EXTENDED 0x93
/* Address High */
/* Address Low  */

/* Definitions for flow control */
#define SIO_RESET          0 /* Reset the port */
#define SIO_MODEM_CTRL     1 /* Set the modem control register */
#define SIO_SET_FLOW_CTRL  2 /* Set flow control register */
#define SIO_SET_BAUD_RATE  3 /* Set baud rate */
#define SIO_SET_DATA       4 /* Set the data characteristics of the port */

#define FTDI_DEVICE_OUT_REQTYPE (LIBUSB_REQUEST_TYPE_VENDOR | LIBUSB_RECIPIENT_DEVICE | LIBUSB_ENDPOINT_OUT)
#define FTDI_DEVICE_IN_REQTYPE (LIBUSB_REQUEST_TYPE_VENDOR | LIBUSB_RECIPIENT_DEVICE | LIBUSB_ENDPOINT_IN)

/* Requests */
#define SIO_RESET_REQUEST             SIO_RESET
#define SIO_SET_BAUDRATE_REQUEST      SIO_SET_BAUD_RATE
#define SIO_SET_DATA_REQUEST          SIO_SET_DATA
#define SIO_SET_FLOW_CTRL_REQUEST     SIO_SET_FLOW_CTRL
#define SIO_SET_MODEM_CTRL_REQUEST    SIO_MODEM_CTRL
#define SIO_POLL_MODEM_STATUS_REQUEST 0x05
#define SIO_SET_EVENT_CHAR_REQUEST    0x06
#define SIO_SET_ERROR_CHAR_REQUEST    0x07
#define SIO_SET_LATENCY_TIMER_REQUEST 0x09
#define SIO_GET_LATENCY_TIMER_REQUEST 0x0A
#define SIO_SET_BITMODE_REQUEST       0x0B
#define SIO_READ_PINS_REQUEST         0x0C
#define SIO_READ_EEPROM_REQUEST       0x90
#define SIO_WRITE_EEPROM_REQUEST      0x91
#define SIO_ERASE_EEPROM_REQUEST      0x92


#define SIO_RESET_SIO 0

/* ** WARNING ** SIO_RESET_PURGE_RX or SIO_RESET_PURGE_TX are values used
 * internally by libftdi to purge the RX and/or TX FIFOs (buffers).
 * APPLICATION PROGRAMS SHOULD NOT BE USING THESE VALUES. Application
 * programs should use one of the ftdi_tciflush, ftdi_tcoflush, or
 * ftdi_tcioflush functions which emulate the Linux serial port tcflush(3)
 * function.
 *
 * History:
 *
 * The definitions for these values are with respect to the FTDI chip, not the
 * CPU. That is, when the FTDI chip receives a USB control transfer request
 * with the command SIO_RESET_PURGE_RX, the FTDI chip empties the FIFO
 * containing data received from the CPU awaiting transfer out the serial
 * port to the connected serial device (e.g., a modem). Likewise, upon
 * reception of the SIO_RESET_PURGE_TX command, the FTDI chip empties the
 * FIFO of data received from the attached serial device destined to be
 * transmitted to the CPU.
 *
 * Unfortunately the coding of the previous releases of libfti assumed these
 * commands had the opposite effect. This resulted in the function
 * ftdi_usb_purge_tx_buffer clearing data received from the attached serial
 * device.  Similarly, the function ftdi_usb_purge_rx_buffer cleared the
 * FTDI FIFO containing data to be transmitted to the attached serial
 * device.  More seriously, this latter function clear the libftid's
 * internal buffer of data received from the serial device, destined
 * to the application program.
 */
#ifdef __GNUC__
#define SIO_RESET_PURGE_RX _Ftdi_Pragma("GCC warning \"SIO_RESET_PURGE_RX\" deprecated: - use tciflush() method") 1
#define SIO_RESET_PURGE_TX _Ftdi_Pragma("GCC warning \"SIO_RESET_PURGE_RX\" deprecated: - use tcoflush() method") 2
#else
#pragma message("WARNING: You need to implement deprecated #define for this compiler")
#define SIO_RESET_PURGE_RX 1
#define SIO_RESET_PURGE_TX 2
#endif
/* New names for the values used internally to flush (purge). */
#define SIO_TCIFLUSH 2
#define SIO_TCOFLUSH 1

#define SIO_DISABLE_FLOW_CTRL 0x0
#define SIO_RTS_CTS_HS (0x1 << 8)
#define SIO_DTR_DSR_HS (0x2 << 8)
#define SIO_XON_XOFF_HS (0x4 << 8)

#define SIO_SET_DTR_MASK 0x1
#define SIO_SET_DTR_HIGH ( 1 | ( SIO_SET_DTR_MASK  << 8))
#define SIO_SET_DTR_LOW  ( 0 | ( SIO_SET_DTR_MASK  << 8))
#define SIO_SET_RTS_MASK 0x2
#define SIO_SET_RTS_HIGH ( 2 | ( SIO_SET_RTS_MASK << 8 ))
#define SIO_SET_RTS_LOW ( 0 | ( SIO_SET_RTS_MASK << 8 ))

#define SIO_RTS_CTS_HS (0x1 << 8)

/* marker for unused usb urb structures
   (taken from libusb) */
#define FTDI_URB_USERCONTEXT_COOKIE ((void *)0x1)

#ifdef _FTDI_DISABLE_DEPRECATED
#define DEPRECATED(func) func
#else
#ifdef __GNUC__
#define DEPRECATED(func) __attribute__ ((deprecated)) func
#elif defined(_MSC_VER)
#define DEPRECATED(func) __declspec(deprecated) func
#else
#pragma message("WARNING: You need to implement DEPRECATED for this compiler")
#define DEPRECATED(func) func
#endif
#endif

struct ftdi_transfer_control
{
    int completed;
    unsigned char *buf;
    int size;
    int offset;
    struct ftdi_context *ftdi;
    struct libusb_transfer *transfer;
};

/**
    \brief Main context structure for all libftdi functions.

    Do not access directly if possible.
*/
struct ftdi_context
{
    /* USB specific */
    /** libusb's context */
    struct libusb_context *usb_ctx;
    /** libusb's usb_dev_handle */
    struct libusb_device_handle *usb_dev;
    /** usb read timeout */
    int usb_read_timeout;
    /** usb write timeout */
    int usb_write_timeout;

    /* FTDI specific */
    /** FTDI chip type */
    enum ftdi_chip_type type;
    /** baudrate */
    int baudrate;
    /** bitbang mode state */
    unsigned char bitbang_enabled;
    /** pointer to read buffer for ftdi_read_data */
    unsigned char *readbuffer;
    /** read buffer offset */
    unsigned int readbuffer_offset;
    /** number of remaining data in internal read buffer */
    unsigned int readbuffer_remaining;
    /** read buffer chunk size */
    unsigned int readbuffer_chunksize;
    /** write buffer chunk size */
    unsigned int writebuffer_chunksize;
    /** maximum packet size. Needed for filtering modem status bytes every n packets. */
    unsigned int max_packet_size;

    /* FTDI FT2232C requirecments */
    /** FT2232C interface number: 0 or 1 */
    int interface;   /* 0 or 1 */
    /** FT2232C index number: 1 or 2 */
    int index;       /* 1 or 2 */
    /* Endpoints */
    /** FT2232C end points: 1 or 2 */
    int in_ep;
    int out_ep;      /* 1 or 2 */

    /** Bitbang mode. 1: (default) Normal bitbang mode, 2: FT2232C SPI bitbang mode */
    unsigned char bitbang_mode;

    /** Decoded eeprom structure */
    struct ftdi_eeprom *eeprom;

    /** String representation of last error */
    const char *error_str;

    /** Defines behavior in case a kernel module is already attached to the device */
    enum ftdi_module_detach_mode module_detach_mode;
};

/**
 List all handled EEPROM values.
   Append future new values only at the end to provide API/ABI stability*/
enum ftdi_eeprom_value
{
    VENDOR_ID          = 0,
    PRODUCT_ID         = 1,
    SELF_POWERED       = 2,
    REMOTE_WAKEUP      = 3,
    IS_NOT_PNP         = 4,
    SUSPEND_DBUS7      = 5,
    IN_IS_ISOCHRONOUS  = 6,
    OUT_IS_ISOCHRONOUS = 7,
    SUSPEND_PULL_DOWNS = 8,
    USE_SERIAL         = 9,
    USB_VERSION        = 10,
    USE_USB_VERSION    = 11,
    MAX_POWER          = 12,
    CHANNEL_A_TYPE     = 13,
    CHANNEL_B_TYPE     = 14,
    CHANNEL_A_DRIVER   = 15,
    CHANNEL_B_DRIVER   = 16,
    CBUS_FUNCTION_0    = 17,
    CBUS_FUNCTION_1    = 18,
    CBUS_FUNCTION_2    = 19,
    CBUS_FUNCTION_3    = 20,
    CBUS_FUNCTION_4    = 21,
    CBUS_FUNCTION_5    = 22,
    CBUS_FUNCTION_6    = 23,
    CBUS_FUNCTION_7    = 24,
    CBUS_FUNCTION_8    = 25,
    CBUS_FUNCTION_9    = 26,
    HIGH_CURRENT       = 27,
    HIGH_CURRENT_A     = 28,
    HIGH_CURRENT_B     = 29,
    INVERT             = 30,
    GROUP0_DRIVE       = 31,
    GROUP0_SCHMITT     = 32,
    GROUP0_SLEW        = 33,
    GROUP1_DRIVE       = 34,
    GROUP1_SCHMITT     = 35,
    GROUP1_SLEW        = 36,
    GROUP2_DRIVE       = 37,
    GROUP2_SCHMITT     = 38,
    GROUP2_SLEW        = 39,
    GROUP3_DRIVE       = 40,
    GROUP3_SCHMITT     = 41,
    GROUP3_SLEW        = 42,
    CHIP_SIZE          = 43,
    CHIP_TYPE          = 44,
    POWER_SAVE         = 45,
    CLOCK_POLARITY     = 46,
    DATA_ORDER         = 47,
    FLOW_CONTROL       = 48,
    CHANNEL_C_DRIVER   = 49,
    CHANNEL_D_DRIVER   = 50,
    CHANNEL_A_RS485    = 51,
    CHANNEL_B_RS485    = 52,
    CHANNEL_C_RS485    = 53,
    CHANNEL_D_RS485    = 54,
    RELEASE_NUMBER     = 55,
    EXTERNAL_OSCILLATOR= 56,
    USER_DATA_ADDR     = 57,
};

/**
    \brief list of usb devices created by ftdi_usb_find_all()
*/
struct ftdi_device_list
{
    /** pointer to next entry */
    struct ftdi_device_list *next;
    /** pointer to libusb's usb_device */
    struct libusb_device *dev;
};
#define FT1284_CLK_IDLE_STATE 0x01
#define FT1284_DATA_LSB       0x02 /* DS_FT232H 1.3 amd ftd2xx.h 1.0.4 disagree here*/
#define FT1284_FLOW_CONTROL   0x04
#define POWER_SAVE_DISABLE_H 0x80

#define USE_SERIAL_NUM 0x08
enum ftdi_cbus_func
{
    CBUS_TXDEN = 0, CBUS_PWREN = 1, CBUS_RXLED = 2, CBUS_TXLED = 3, CBUS_TXRXLED = 4,
    CBUS_SLEEP = 5, CBUS_CLK48 = 6, CBUS_CLK24 = 7, CBUS_CLK12 = 8, CBUS_CLK6 =  9,
    CBUS_IOMODE = 0xa, CBUS_BB_WR = 0xb, CBUS_BB_RD = 0xc
};

enum ftdi_cbush_func
{
    CBUSH_TRISTATE = 0, CBUSH_TXLED = 1, CBUSH_RXLED = 2, CBUSH_TXRXLED = 3, CBUSH_PWREN = 4,
    CBUSH_SLEEP = 5, CBUSH_DRIVE_0 = 6, CBUSH_DRIVE1 = 7, CBUSH_IOMODE = 8, CBUSH_TXDEN =  9,
    CBUSH_CLK30 = 10, CBUSH_CLK15 = 11, CBUSH_CLK7_5 = 12
};

enum ftdi_cbusx_func
{
    CBUSX_TRISTATE = 0, CBUSX_TXLED = 1, CBUSX_RXLED = 2, CBUSX_TXRXLED = 3, CBUSX_PWREN = 4,
    CBUSX_SLEEP = 5, CBUSX_DRIVE_0 = 6, CBUSX_DRIVE1 = 7, CBUSX_IOMODE = 8, CBUSX_TXDEN =  9,
    CBUSX_CLK24 = 10, CBUSX_CLK12 = 11, CBUSX_CLK6 = 12, CBUSX_BAT_DETECT = 13,
    CBUSX_BAT_DETECT_NEG = 14, CBUSX_I2C_TXE = 15, CBUSX_I2C_RXF = 16, CBUSX_VBUS_SENSE = 17,
    CBUSX_BB_WR = 18, CBUSX_BB_RD = 19, CBUSX_TIME_STAMP = 20, CBUSX_AWAKE = 21
};

/** Invert TXD# */
#define INVERT_TXD 0x01
/** Invert RXD# */
#define INVERT_RXD 0x02
/** Invert RTS# */
#define INVERT_RTS 0x04
/** Invert CTS# */
#define INVERT_CTS 0x08
/** Invert DTR# */
#define INVERT_DTR 0x10
/** Invert DSR# */
#define INVERT_DSR 0x20
/** Invert DCD# */
#define INVERT_DCD 0x40
/** Invert RI# */
#define INVERT_RI  0x80

/** Interface Mode. */
#define CHANNEL_IS_UART 0x0
#define CHANNEL_IS_FIFO 0x1
#define CHANNEL_IS_OPTO 0x2
#define CHANNEL_IS_CPU  0x4
#define CHANNEL_IS_FT1284 0x8

#define CHANNEL_IS_RS485 0x10

#define DRIVE_4MA  0
#define DRIVE_8MA  1
#define DRIVE_12MA 2
#define DRIVE_16MA 3
#define SLOW_SLEW  4
#define IS_SCHMITT 8

/** Driver Type. */
#define DRIVER_VCP 0x08
#define DRIVER_VCPH 0x10 /* FT232H has moved the VCP bit */

#define USE_USB_VERSION_BIT 0x10

#define SUSPEND_DBUS7_BIT 0x80

/** High current drive. */
#define HIGH_CURRENT_DRIVE   0x10
#define HIGH_CURRENT_DRIVE_R 0x04

/**
    \brief Progress Info for streaming read
*/
struct size_and_time
{
    uint64_t totalBytes;
    struct timeval time;
};

typedef struct
{
    struct size_and_time first;
    struct size_and_time prev;
    struct size_and_time current;
    double totalTime;
    double totalRate;
    double currentRate;
} FTDIProgressInfo;

typedef int (FTDIStreamCallback)(uint8_t *buffer, int length,
                                 FTDIProgressInfo *progress, void *userdata);

/**
 * Provide libftdi version information
 * major: Library major version
 * minor: Library minor version
 * micro: Currently unused, ight get used for hotfixes.
 * version_str: Version as (static) string
 * snapshot_str: Git snapshot version if known. Otherwise "unknown" or empty string.
*/
struct ftdi_version_info
{
    int major;
    int minor;
    int micro;
    const char *version_str;
    const char *snapshot_str;
};


#ifdef __cplusplus
extern "C"
{
#endif

    int ftdi_init(struct ftdi_context *ftdi);
    struct ftdi_context *ftdi_new(void);
    int ftdi_set_interface(struct ftdi_context *ftdi, enum ftdi_interface interface);

    void ftdi_deinit(struct ftdi_context *ftdi);
    void ftdi_free(struct ftdi_context *ftdi);
    void ftdi_set_usbdev (struct ftdi_context *ftdi, struct libusb_device_handle *usbdev);

    struct ftdi_version_info ftdi_get_library_version(void);

    int ftdi_usb_find_all(struct ftdi_context *ftdi, struct ftdi_device_list **devlist,
                          int vendor, int product);
    void ftdi_list_free(struct ftdi_device_list **devlist);
    void ftdi_list_free2(struct ftdi_device_list *devlist);
    int ftdi_usb_get_strings(struct ftdi_context *ftdi, struct libusb_device *dev,
                             char *manufacturer, int mnf_len,
                             char *description, int desc_len,
                             char *serial, int serial_len);
    int ftdi_usb_get_strings2(struct ftdi_context *ftdi, struct libusb_device *dev,
                              char *manufacturer, int mnf_len,
                              char *description, int desc_len,
                              char *serial, int serial_len);

    int ftdi_eeprom_get_strings(struct ftdi_context *ftdi,
                                char *manufacturer, int mnf_len,
                                char *product, int prod_len,
                                char *serial, int serial_len);
    int ftdi_eeprom_set_strings(struct ftdi_context *ftdi, const char * manufacturer,
                                const char * product, const char * serial);

    int ftdi_usb_open(struct ftdi_context *ftdi, int vendor, int product);
    int ftdi_usb_open_desc(struct ftdi_context *ftdi, int vendor, int product,
                           const char* description, const char* serial);
    int ftdi_usb_open_desc_index(struct ftdi_context *ftdi, int vendor, int product,
                                 const char* description, const char* serial, unsigned int index);
    int ftdi_usb_open_bus_addr(struct ftdi_context *ftdi, uint8_t bus, uint8_t addr);
    int ftdi_usb_open_dev(struct ftdi_context *ftdi, struct libusb_device *dev);
    int ftdi_usb_open_string(struct ftdi_context *ftdi, const char* description);

    int ftdi_usb_close(struct ftdi_context *ftdi);
    int ftdi_usb_reset(struct ftdi_context *ftdi);
    int ftdi_tciflush(struct ftdi_context *ftdi);
    int ftdi_tcoflush(struct ftdi_context *ftdi);
    int ftdi_tcioflush(struct ftdi_context *ftdi);
    int DEPRECATED(ftdi_usb_purge_rx_buffer(struct ftdi_context *ftdi));
    int DEPRECATED(ftdi_usb_purge_tx_buffer(struct ftdi_context *ftdi));
    int DEPRECATED(ftdi_usb_purge_buffers(struct ftdi_context *ftdi));

    int ftdi_set_baudrate(struct ftdi_context *ftdi, int baudrate);
    int ftdi_set_line_property(struct ftdi_context *ftdi, enum ftdi_bits_type bits,
                               enum ftdi_stopbits_type sbit, enum ftdi_parity_type parity);
    int ftdi_set_line_property2(struct ftdi_context *ftdi, enum ftdi_bits_type bits,
                                enum ftdi_stopbits_type sbit, enum ftdi_parity_type parity,
                                enum ftdi_break_type break_type);

    int ftdi_read_data(struct ftdi_context *ftdi, unsigned char *buf, int size);
    int ftdi_read_data_set_chunksize(struct ftdi_context *ftdi, unsigned int chunksize);
    int ftdi_read_data_get_chunksize(struct ftdi_context *ftdi, unsigned int *chunksize);

    int ftdi_write_data(struct ftdi_context *ftdi, const unsigned char *buf, int size);
    int ftdi_write_data_set_chunksize(struct ftdi_context *ftdi, unsigned int chunksize);
    int ftdi_write_data_get_chunksize(struct ftdi_context *ftdi, unsigned int *chunksize);

    int ftdi_readstream(struct ftdi_context *ftdi, FTDIStreamCallback *callback,
                        void *userdata, int packetsPerTransfer, int numTransfers);
    struct ftdi_transfer_control *ftdi_write_data_submit(struct ftdi_context *ftdi, unsigned char *buf, int size);

    struct ftdi_transfer_control *ftdi_read_data_submit(struct ftdi_context *ftdi, unsigned char *buf, int size);
    int ftdi_transfer_data_done(struct ftdi_transfer_control *tc);
    void ftdi_transfer_data_cancel(struct ftdi_transfer_control *tc, struct timeval * to);

    int ftdi_set_bitmode(struct ftdi_context *ftdi, unsigned char bitmask, unsigned char mode);
    int ftdi_disable_bitbang(struct ftdi_context *ftdi);
    int ftdi_read_pins(struct ftdi_context *ftdi, unsigned char *pins);

    int ftdi_set_latency_timer(struct ftdi_context *ftdi, unsigned char latency);
    int ftdi_get_latency_timer(struct ftdi_context *ftdi, unsigned char *latency);

    int ftdi_poll_modem_status(struct ftdi_context *ftdi, unsigned short *status);

    /* flow control */
    int ftdi_setflowctrl(struct ftdi_context *ftdi, int flowctrl);
    int ftdi_setflowctrl_xonxoff(struct ftdi_context *ftdi, unsigned char xon, unsigned char xoff);
    int ftdi_setdtr_rts(struct ftdi_context *ftdi, int dtr, int rts);
    int ftdi_setdtr(struct ftdi_context *ftdi, int state);
    int ftdi_setrts(struct ftdi_context *ftdi, int state);

    int ftdi_set_event_char(struct ftdi_context *ftdi, unsigned char eventch, unsigned char enable);
    int ftdi_set_error_char(struct ftdi_context *ftdi, unsigned char errorch, unsigned char enable);

    /* init eeprom for the given FTDI type */
    int ftdi_eeprom_initdefaults(struct ftdi_context *ftdi,
                                 char * manufacturer, char *product,
                                 char * serial);
    int ftdi_eeprom_build(struct ftdi_context *ftdi);
    int ftdi_eeprom_decode(struct ftdi_context *ftdi, int verbose);

    int ftdi_get_eeprom_value(struct ftdi_context *ftdi, enum ftdi_eeprom_value value_name, int* value);
    int ftdi_set_eeprom_value(struct ftdi_context *ftdi, enum ftdi_eeprom_value value_name, int  value);

    int ftdi_get_eeprom_buf(struct ftdi_context *ftdi, unsigned char * buf, int size);
    int ftdi_set_eeprom_buf(struct ftdi_context *ftdi, const unsigned char * buf, int size);

    int ftdi_set_eeprom_user_data(struct ftdi_context *ftdi, const char * buf, int size);

    int ftdi_read_eeprom(struct ftdi_context *ftdi);
    int ftdi_read_chipid(struct ftdi_context *ftdi, unsigned int *chipid);
    int ftdi_write_eeprom(struct ftdi_context *ftdi);
    int ftdi_erase_eeprom(struct ftdi_context *ftdi);

    int ftdi_read_eeprom_location (struct ftdi_context *ftdi, int eeprom_addr, unsigned short *eeprom_val);
    int ftdi_write_eeprom_location(struct ftdi_context *ftdi, int eeprom_addr, unsigned short eeprom_val);

    const char *ftdi_get_error_string(struct ftdi_context *ftdi);

#ifdef __cplusplus
}
#endif

#endif /* __libftdi_h__ */
