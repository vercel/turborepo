/*
 * xusb: Generic USB test program
 * Copyright © 2009-2012 Pete Batard <pete@akeo.ie>
 * Contributions to Mass Storage by Alan Stern.
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, write to the Free Software
 * Foundation, Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA
 */

#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>

#include "libusb.h"

#if defined(_WIN32)
#define msleep(msecs) Sleep(msecs)
#else
#include <time.h>
#define msleep(msecs) nanosleep(&(struct timespec){msecs / 1000, (msecs * 1000000) % 1000000000UL}, NULL);
#endif

#if defined(_MSC_VER)
#define snprintf _snprintf
#define putenv _putenv
#endif

#if !defined(bool)
#define bool int
#endif
#if !defined(true)
#define true (1 == 1)
#endif
#if !defined(false)
#define false (!true)
#endif

// Future versions of libusb will use usb_interface instead of interface
// in libusb_config_descriptor => catter for that
#define usb_interface interface

// Global variables
static bool binary_dump = false;
static bool extra_info = false;
static bool force_device_request = false;	// For WCID descriptor queries
static const char* binary_name = NULL;

static void perr(char const *format, ...)
{
	va_list args;

	va_start (args, format);
	vfprintf(stderr, format, args);
	va_end(args);
}

#define ERR_EXIT(errcode) do { perr("   %s\n", libusb_strerror((enum libusb_error)errcode)); return -1; } while (0)
#define CALL_CHECK(fcall) do { int _r=fcall; if (_r < 0) ERR_EXIT(_r); } while (0)
#define CALL_CHECK_CLOSE(fcall, hdl) do { int _r=fcall; if (_r < 0) { libusb_close(hdl); ERR_EXIT(_r); } } while (0)
#define B(x) (((x)!=0)?1:0)
#define be_to_int32(buf) (((buf)[0]<<24)|((buf)[1]<<16)|((buf)[2]<<8)|(buf)[3])

#define RETRY_MAX                     5
#define REQUEST_SENSE_LENGTH          0x12
#define INQUIRY_LENGTH                0x24
#define READ_CAPACITY_LENGTH          0x08

// HID Class-Specific Requests values. See section 7.2 of the HID specifications
#define HID_GET_REPORT                0x01
#define HID_GET_IDLE                  0x02
#define HID_GET_PROTOCOL              0x03
#define HID_SET_REPORT                0x09
#define HID_SET_IDLE                  0x0A
#define HID_SET_PROTOCOL              0x0B
#define HID_REPORT_TYPE_INPUT         0x01
#define HID_REPORT_TYPE_OUTPUT        0x02
#define HID_REPORT_TYPE_FEATURE       0x03

// Mass Storage Requests values. See section 3 of the Bulk-Only Mass Storage Class specifications
#define BOMS_RESET                    0xFF
#define BOMS_GET_MAX_LUN              0xFE

// Microsoft OS Descriptor
#define MS_OS_DESC_STRING_INDEX		0xEE
#define MS_OS_DESC_STRING_LENGTH	0x12
#define MS_OS_DESC_VENDOR_CODE_OFFSET	0x10
static const uint8_t ms_os_desc_string[] = {
	MS_OS_DESC_STRING_LENGTH,
	LIBUSB_DT_STRING,
	'M', 0, 'S', 0, 'F', 0, 'T', 0, '1', 0, '0', 0, '0', 0,
};

// Section 5.1: Command Block Wrapper (CBW)
struct command_block_wrapper {
	uint8_t dCBWSignature[4];
	uint32_t dCBWTag;
	uint32_t dCBWDataTransferLength;
	uint8_t bmCBWFlags;
	uint8_t bCBWLUN;
	uint8_t bCBWCBLength;
	uint8_t CBWCB[16];
};

// Section 5.2: Command Status Wrapper (CSW)
struct command_status_wrapper {
	uint8_t dCSWSignature[4];
	uint32_t dCSWTag;
	uint32_t dCSWDataResidue;
	uint8_t bCSWStatus;
};

static const uint8_t cdb_length[256] = {
//	 0  1  2  3  4  5  6  7  8  9  A  B  C  D  E  F
	06,06,06,06,06,06,06,06,06,06,06,06,06,06,06,06,  //  0
	06,06,06,06,06,06,06,06,06,06,06,06,06,06,06,06,  //  1
	10,10,10,10,10,10,10,10,10,10,10,10,10,10,10,10,  //  2
	10,10,10,10,10,10,10,10,10,10,10,10,10,10,10,10,  //  3
	10,10,10,10,10,10,10,10,10,10,10,10,10,10,10,10,  //  4
	10,10,10,10,10,10,10,10,10,10,10,10,10,10,10,10,  //  5
	00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,  //  6
	00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,  //  7
	16,16,16,16,16,16,16,16,16,16,16,16,16,16,16,16,  //  8
	16,16,16,16,16,16,16,16,16,16,16,16,16,16,16,16,  //  9
	12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,  //  A
	12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,  //  B
	00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,  //  C
	00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,  //  D
	00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,  //  E
	00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,  //  F
};

static enum test_type {
	USE_GENERIC,
	USE_PS3,
	USE_XBOX,
	USE_SCSI,
	USE_HID,
} test_mode;
static uint16_t VID, PID;

static void display_buffer_hex(unsigned char *buffer, unsigned size)
{
	unsigned i, j, k;

	for (i=0; i<size; i+=16) {
		printf("\n  %08x  ", i);
		for(j=0,k=0; k<16; j++,k++) {
			if (i+j < size) {
				printf("%02x", buffer[i+j]);
			} else {
				printf("  ");
			}
			printf(" ");
		}
		printf(" ");
		for(j=0,k=0; k<16; j++,k++) {
			if (i+j < size) {
				if ((buffer[i+j] < 32) || (buffer[i+j] > 126)) {
					printf(".");
				} else {
					printf("%c", buffer[i+j]);
				}
			}
		}
	}
	printf("\n" );
}

static char* uuid_to_string(const uint8_t* uuid)
{
	static char uuid_string[40];
	if (uuid == NULL) return NULL;
	snprintf(uuid_string, sizeof(uuid_string),
		"{%02x%02x%02x%02x-%02x%02x-%02x%02x-%02x%02x-%02x%02x%02x%02x%02x%02x}",
		uuid[0], uuid[1], uuid[2], uuid[3], uuid[4], uuid[5], uuid[6], uuid[7],
		uuid[8], uuid[9], uuid[10], uuid[11], uuid[12], uuid[13], uuid[14], uuid[15]);
	return uuid_string;
}

// The PS3 Controller is really a HID device that got its HID Report Descriptors
// removed by Sony
static int display_ps3_status(libusb_device_handle *handle)
{
	uint8_t input_report[49];
	uint8_t master_bt_address[8];
	uint8_t device_bt_address[18];

	// Get the controller's bluetooth address of its master device
	CALL_CHECK(libusb_control_transfer(handle, LIBUSB_ENDPOINT_IN|LIBUSB_REQUEST_TYPE_CLASS|LIBUSB_RECIPIENT_INTERFACE,
		HID_GET_REPORT, 0x03f5, 0, master_bt_address, sizeof(master_bt_address), 100));
	printf("\nMaster's bluetooth address: %02X:%02X:%02X:%02X:%02X:%02X\n", master_bt_address[2], master_bt_address[3],
		master_bt_address[4], master_bt_address[5], master_bt_address[6], master_bt_address[7]);

	// Get the controller's bluetooth address
	CALL_CHECK(libusb_control_transfer(handle, LIBUSB_ENDPOINT_IN|LIBUSB_REQUEST_TYPE_CLASS|LIBUSB_RECIPIENT_INTERFACE,
		HID_GET_REPORT, 0x03f2, 0, device_bt_address, sizeof(device_bt_address), 100));
	printf("\nMaster's bluetooth address: %02X:%02X:%02X:%02X:%02X:%02X\n", device_bt_address[4], device_bt_address[5],
		device_bt_address[6], device_bt_address[7], device_bt_address[8], device_bt_address[9]);

	// Get the status of the controller's buttons via its HID report
	printf("\nReading PS3 Input Report...\n");
	CALL_CHECK(libusb_control_transfer(handle, LIBUSB_ENDPOINT_IN|LIBUSB_REQUEST_TYPE_CLASS|LIBUSB_RECIPIENT_INTERFACE,
		HID_GET_REPORT, (HID_REPORT_TYPE_INPUT<<8)|0x01, 0, input_report, sizeof(input_report), 1000));
	switch(input_report[2]){	/** Direction pad plus start, select, and joystick buttons */
		case 0x01:
			printf("\tSELECT pressed\n");
			break;
		case 0x02:
			printf("\tLEFT 3 pressed\n");
			break;
		case 0x04:
			printf("\tRIGHT 3 pressed\n");
			break;
		case 0x08:
			printf("\tSTART presed\n");
			break;
		case 0x10:
			printf("\tUP pressed\n");
			break;
		case 0x20:
			printf("\tRIGHT pressed\n");
			break;
		case 0x40:
			printf("\tDOWN pressed\n");
			break;
		case 0x80:
			printf("\tLEFT pressed\n");
			break;
	}
	switch(input_report[3]){	/** Shapes plus top right and left buttons */
		case 0x01:
			printf("\tLEFT 2 pressed\n");
			break;
		case 0x02:
			printf("\tRIGHT 2 pressed\n");
			break;
		case 0x04:
			printf("\tLEFT 1 pressed\n");
			break;
		case 0x08:
			printf("\tRIGHT 1 presed\n");
			break;
		case 0x10:
			printf("\tTRIANGLE pressed\n");
			break;
		case 0x20:
			printf("\tCIRCLE pressed\n");
			break;
		case 0x40:
			printf("\tCROSS pressed\n");
			break;
		case 0x80:
			printf("\tSQUARE pressed\n");
			break;
	}
	printf("\tPS button: %d\n", input_report[4]);
	printf("\tLeft Analog (X,Y): (%d,%d)\n", input_report[6], input_report[7]);
	printf("\tRight Analog (X,Y): (%d,%d)\n", input_report[8], input_report[9]);
	printf("\tL2 Value: %d\tR2 Value: %d\n", input_report[18], input_report[19]);
	printf("\tL1 Value: %d\tR1 Value: %d\n", input_report[20], input_report[21]);
	printf("\tRoll (x axis): %d Yaw (y axis): %d Pitch (z axis) %d\n",
			//(((input_report[42] + 128) % 256) - 128),
			(int8_t)(input_report[42]),
			(int8_t)(input_report[44]),
			(int8_t)(input_report[46]));
	printf("\tAcceleration: %d\n\n", (int8_t)(input_report[48]));
	return 0;
}
// The XBOX Controller is really a HID device that got its HID Report Descriptors
// removed by Microsoft.
// Input/Output reports described at http://euc.jp/periphs/xbox-controller.ja.html
static int display_xbox_status(libusb_device_handle *handle)
{
	uint8_t input_report[20];
	printf("\nReading XBox Input Report...\n");
	CALL_CHECK(libusb_control_transfer(handle, LIBUSB_ENDPOINT_IN|LIBUSB_REQUEST_TYPE_CLASS|LIBUSB_RECIPIENT_INTERFACE,
		HID_GET_REPORT, (HID_REPORT_TYPE_INPUT<<8)|0x00, 0, input_report, 20, 1000));
	printf("   D-pad: %02X\n", input_report[2]&0x0F);
	printf("   Start:%d, Back:%d, Left Stick Press:%d, Right Stick Press:%d\n", B(input_report[2]&0x10), B(input_report[2]&0x20),
		B(input_report[2]&0x40), B(input_report[2]&0x80));
	// A, B, X, Y, Black, White are pressure sensitive
	printf("   A:%d, B:%d, X:%d, Y:%d, White:%d, Black:%d\n", input_report[4], input_report[5],
		input_report[6], input_report[7], input_report[9], input_report[8]);
	printf("   Left Trigger: %d, Right Trigger: %d\n", input_report[10], input_report[11]);
	printf("   Left Analog (X,Y): (%d,%d)\n", (int16_t)((input_report[13]<<8)|input_report[12]),
		(int16_t)((input_report[15]<<8)|input_report[14]));
	printf("   Right Analog (X,Y): (%d,%d)\n", (int16_t)((input_report[17]<<8)|input_report[16]),
		(int16_t)((input_report[19]<<8)|input_report[18]));
	return 0;
}

static int set_xbox_actuators(libusb_device_handle *handle, uint8_t left, uint8_t right)
{
	uint8_t output_report[6];

	printf("\nWriting XBox Controller Output Report...\n");

	memset(output_report, 0, sizeof(output_report));
	output_report[1] = sizeof(output_report);
	output_report[3] = left;
	output_report[5] = right;

	CALL_CHECK(libusb_control_transfer(handle, LIBUSB_ENDPOINT_OUT|LIBUSB_REQUEST_TYPE_CLASS|LIBUSB_RECIPIENT_INTERFACE,
		HID_SET_REPORT, (HID_REPORT_TYPE_OUTPUT<<8)|0x00, 0, output_report, 06, 1000));
	return 0;
}

static int send_mass_storage_command(libusb_device_handle *handle, uint8_t endpoint, uint8_t lun,
	uint8_t *cdb, uint8_t direction, int data_length, uint32_t *ret_tag)
{
	static uint32_t tag = 1;
	uint8_t cdb_len;
	int i, r, size;
	struct command_block_wrapper cbw;

	if (cdb == NULL) {
		return -1;
	}

	if (endpoint & LIBUSB_ENDPOINT_IN) {
		perr("send_mass_storage_command: cannot send command on IN endpoint\n");
		return -1;
	}

	cdb_len = cdb_length[cdb[0]];
	if ((cdb_len == 0) || (cdb_len > sizeof(cbw.CBWCB))) {
		perr("send_mass_storage_command: don't know how to handle this command (%02X, length %d)\n",
			cdb[0], cdb_len);
		return -1;
	}

	memset(&cbw, 0, sizeof(cbw));
	cbw.dCBWSignature[0] = 'U';
	cbw.dCBWSignature[1] = 'S';
	cbw.dCBWSignature[2] = 'B';
	cbw.dCBWSignature[3] = 'C';
	*ret_tag = tag;
	cbw.dCBWTag = tag++;
	cbw.dCBWDataTransferLength = data_length;
	cbw.bmCBWFlags = direction;
	cbw.bCBWLUN = lun;
	// Subclass is 1 or 6 => cdb_len
	cbw.bCBWCBLength = cdb_len;
	memcpy(cbw.CBWCB, cdb, cdb_len);

	i = 0;
	do {
		// The transfer length must always be exactly 31 bytes.
		r = libusb_bulk_transfer(handle, endpoint, (unsigned char*)&cbw, 31, &size, 1000);
		if (r == LIBUSB_ERROR_PIPE) {
			libusb_clear_halt(handle, endpoint);
		}
		i++;
	} while ((r == LIBUSB_ERROR_PIPE) && (i<RETRY_MAX));
	if (r != LIBUSB_SUCCESS) {
		perr("   send_mass_storage_command: %s\n", libusb_strerror((enum libusb_error)r));
		return -1;
	}

	printf("   sent %d CDB bytes\n", cdb_len);
	return 0;
}

static int get_mass_storage_status(libusb_device_handle *handle, uint8_t endpoint, uint32_t expected_tag)
{
	int i, r, size;
	struct command_status_wrapper csw;

	// The device is allowed to STALL this transfer. If it does, you have to
	// clear the stall and try again.
	i = 0;
	do {
		r = libusb_bulk_transfer(handle, endpoint, (unsigned char*)&csw, 13, &size, 1000);
		if (r == LIBUSB_ERROR_PIPE) {
			libusb_clear_halt(handle, endpoint);
		}
		i++;
	} while ((r == LIBUSB_ERROR_PIPE) && (i<RETRY_MAX));
	if (r != LIBUSB_SUCCESS) {
		perr("   get_mass_storage_status: %s\n", libusb_strerror((enum libusb_error)r));
		return -1;
	}
	if (size != 13) {
		perr("   get_mass_storage_status: received %d bytes (expected 13)\n", size);
		return -1;
	}
	if (csw.dCSWTag != expected_tag) {
		perr("   get_mass_storage_status: mismatched tags (expected %08X, received %08X)\n",
			expected_tag, csw.dCSWTag);
		return -1;
	}
	// For this test, we ignore the dCSWSignature check for validity...
	printf("   Mass Storage Status: %02X (%s)\n", csw.bCSWStatus, csw.bCSWStatus?"FAILED":"Success");
	if (csw.dCSWTag != expected_tag)
		return -1;
	if (csw.bCSWStatus) {
		// REQUEST SENSE is appropriate only if bCSWStatus is 1, meaning that the
		// command failed somehow.  Larger values (2 in particular) mean that
		// the command couldn't be understood.
		if (csw.bCSWStatus == 1)
			return -2;	// request Get Sense
		else
			return -1;
	}

	// In theory we also should check dCSWDataResidue.  But lots of devices
	// set it wrongly.
	return 0;
}

static void get_sense(libusb_device_handle *handle, uint8_t endpoint_in, uint8_t endpoint_out)
{
	uint8_t cdb[16];	// SCSI Command Descriptor Block
	uint8_t sense[18];
	uint32_t expected_tag;
	int size;
	int rc;

	// Request Sense
	printf("Request Sense:\n");
	memset(sense, 0, sizeof(sense));
	memset(cdb, 0, sizeof(cdb));
	cdb[0] = 0x03;	// Request Sense
	cdb[4] = REQUEST_SENSE_LENGTH;

	send_mass_storage_command(handle, endpoint_out, 0, cdb, LIBUSB_ENDPOINT_IN, REQUEST_SENSE_LENGTH, &expected_tag);
	rc = libusb_bulk_transfer(handle, endpoint_in, (unsigned char*)&sense, REQUEST_SENSE_LENGTH, &size, 1000);
	if (rc < 0)
	{
		printf("libusb_bulk_transfer failed: %s\n", libusb_error_name(rc));
		return;
	}
	printf("   received %d bytes\n", size);

	if ((sense[0] != 0x70) && (sense[0] != 0x71)) {
		perr("   ERROR No sense data\n");
	} else {
		perr("   ERROR Sense: %02X %02X %02X\n", sense[2]&0x0F, sense[12], sense[13]);
	}
	// Strictly speaking, the get_mass_storage_status() call should come
	// before these perr() lines.  If the status is nonzero then we must
	// assume there's no data in the buffer.  For xusb it doesn't matter.
	get_mass_storage_status(handle, endpoint_in, expected_tag);
}

// Mass Storage device to test bulk transfers (non destructive test)
static int test_mass_storage(libusb_device_handle *handle, uint8_t endpoint_in, uint8_t endpoint_out)
{
	int r, size;
	uint8_t lun;
	uint32_t expected_tag;
	uint32_t i, max_lba, block_size;
	double device_size;
	uint8_t cdb[16];	// SCSI Command Descriptor Block
	uint8_t buffer[64];
	char vid[9], pid[9], rev[5];
	unsigned char *data;
	FILE *fd;

	printf("Reading Max LUN:\n");
	r = libusb_control_transfer(handle, LIBUSB_ENDPOINT_IN|LIBUSB_REQUEST_TYPE_CLASS|LIBUSB_RECIPIENT_INTERFACE,
		BOMS_GET_MAX_LUN, 0, 0, &lun, 1, 1000);
	// Some devices send a STALL instead of the actual value.
	// In such cases we should set lun to 0.
	if (r == 0) {
		lun = 0;
	} else if (r < 0) {
		perr("   Failed: %s", libusb_strerror((enum libusb_error)r));
	}
	printf("   Max LUN = %d\n", lun);

	// Send Inquiry
	printf("Sending Inquiry:\n");
	memset(buffer, 0, sizeof(buffer));
	memset(cdb, 0, sizeof(cdb));
	cdb[0] = 0x12;	// Inquiry
	cdb[4] = INQUIRY_LENGTH;

	send_mass_storage_command(handle, endpoint_out, lun, cdb, LIBUSB_ENDPOINT_IN, INQUIRY_LENGTH, &expected_tag);
	CALL_CHECK(libusb_bulk_transfer(handle, endpoint_in, (unsigned char*)&buffer, INQUIRY_LENGTH, &size, 1000));
	printf("   received %d bytes\n", size);
	// The following strings are not zero terminated
	for (i=0; i<8; i++) {
		vid[i] = buffer[8+i];
		pid[i] = buffer[16+i];
		rev[i/2] = buffer[32+i/2];	// instead of another loop
	}
	vid[8] = 0;
	pid[8] = 0;
	rev[4] = 0;
	printf("   VID:PID:REV \"%8s\":\"%8s\":\"%4s\"\n", vid, pid, rev);
	if (get_mass_storage_status(handle, endpoint_in, expected_tag) == -2) {
		get_sense(handle, endpoint_in, endpoint_out);
	}

	// Read capacity
	printf("Reading Capacity:\n");
	memset(buffer, 0, sizeof(buffer));
	memset(cdb, 0, sizeof(cdb));
	cdb[0] = 0x25;	// Read Capacity

	send_mass_storage_command(handle, endpoint_out, lun, cdb, LIBUSB_ENDPOINT_IN, READ_CAPACITY_LENGTH, &expected_tag);
	CALL_CHECK(libusb_bulk_transfer(handle, endpoint_in, (unsigned char*)&buffer, READ_CAPACITY_LENGTH, &size, 1000));
	printf("   received %d bytes\n", size);
	max_lba = be_to_int32(&buffer[0]);
	block_size = be_to_int32(&buffer[4]);
	device_size = ((double)(max_lba+1))*block_size/(1024*1024*1024);
	printf("   Max LBA: %08X, Block Size: %08X (%.2f GB)\n", max_lba, block_size, device_size);
	if (get_mass_storage_status(handle, endpoint_in, expected_tag) == -2) {
		get_sense(handle, endpoint_in, endpoint_out);
	}

	// coverity[tainted_data]
	data = (unsigned char*) calloc(1, block_size);
	if (data == NULL) {
		perr("   unable to allocate data buffer\n");
		return -1;
	}

	// Send Read
	printf("Attempting to read %u bytes:\n", block_size);
	memset(cdb, 0, sizeof(cdb));

	cdb[0] = 0x28;	// Read(10)
	cdb[8] = 0x01;	// 1 block

	send_mass_storage_command(handle, endpoint_out, lun, cdb, LIBUSB_ENDPOINT_IN, block_size, &expected_tag);
	libusb_bulk_transfer(handle, endpoint_in, data, block_size, &size, 5000);
	printf("   READ: received %d bytes\n", size);
	if (get_mass_storage_status(handle, endpoint_in, expected_tag) == -2) {
		get_sense(handle, endpoint_in, endpoint_out);
	} else {
		display_buffer_hex(data, size);
		if ((binary_dump) && ((fd = fopen(binary_name, "w")) != NULL)) {
			if (fwrite(data, 1, (size_t)size, fd) != (unsigned int)size) {
				perr("   unable to write binary data\n");
			}
			fclose(fd);
		}
	}
	free(data);

	return 0;
}

// HID
static int get_hid_record_size(uint8_t *hid_report_descriptor, int size, int type)
{
	uint8_t i, j = 0;
	uint8_t offset;
	int record_size[3] = {0, 0, 0};
	int nb_bits = 0, nb_items = 0;
	bool found_record_marker;

	found_record_marker = false;
	for (i = hid_report_descriptor[0]+1; i < size; i += offset) {
		offset = (hid_report_descriptor[i]&0x03) + 1;
		if (offset == 4)
			offset = 5;
		switch (hid_report_descriptor[i] & 0xFC) {
		case 0x74:	// bitsize
			nb_bits = hid_report_descriptor[i+1];
			break;
		case 0x94:	// count
			nb_items = 0;
			for (j=1; j<offset; j++) {
				nb_items = ((uint32_t)hid_report_descriptor[i+j]) << (8*(j-1));
			}
			break;
		case 0x80:	// input
			found_record_marker = true;
			j = 0;
			break;
		case 0x90:	// output
			found_record_marker = true;
			j = 1;
			break;
		case 0xb0:	// feature
			found_record_marker = true;
			j = 2;
			break;
		case 0xC0:	// end of collection
			nb_items = 0;
			nb_bits = 0;
			break;
		default:
			continue;
		}
		if (found_record_marker) {
			found_record_marker = false;
			record_size[j] += nb_items*nb_bits;
		}
	}
	if ((type < HID_REPORT_TYPE_INPUT) || (type > HID_REPORT_TYPE_FEATURE)) {
		return 0;
	} else {
		return (record_size[type - HID_REPORT_TYPE_INPUT]+7)/8;
	}
}

static int test_hid(libusb_device_handle *handle, uint8_t endpoint_in)
{
	int r, size, descriptor_size;
	uint8_t hid_report_descriptor[256];
	uint8_t *report_buffer;
	FILE *fd;

	printf("\nReading HID Report Descriptors:\n");
	descriptor_size = libusb_control_transfer(handle, LIBUSB_ENDPOINT_IN|LIBUSB_REQUEST_TYPE_STANDARD|LIBUSB_RECIPIENT_INTERFACE,
		LIBUSB_REQUEST_GET_DESCRIPTOR, LIBUSB_DT_REPORT<<8, 0, hid_report_descriptor, sizeof(hid_report_descriptor), 1000);
	if (descriptor_size < 0) {
		printf("   Failed\n");
		return -1;
	}
	display_buffer_hex(hid_report_descriptor, descriptor_size);
	if ((binary_dump) && ((fd = fopen(binary_name, "w")) != NULL)) {
		if (fwrite(hid_report_descriptor, 1, descriptor_size, fd) != (size_t)descriptor_size) {
			printf("   Error writing descriptor to file\n");
		}
		fclose(fd);
	}

	size = get_hid_record_size(hid_report_descriptor, descriptor_size, HID_REPORT_TYPE_FEATURE);
	if (size <= 0) {
		printf("\nSkipping Feature Report readout (None detected)\n");
	} else {
		report_buffer = (uint8_t*) calloc(size, 1);
		if (report_buffer == NULL) {
			return -1;
		}

		printf("\nReading Feature Report (length %d)...\n", size);
		r = libusb_control_transfer(handle, LIBUSB_ENDPOINT_IN|LIBUSB_REQUEST_TYPE_CLASS|LIBUSB_RECIPIENT_INTERFACE,
			HID_GET_REPORT, (HID_REPORT_TYPE_FEATURE<<8)|0, 0, report_buffer, (uint16_t)size, 5000);
		if (r >= 0) {
			display_buffer_hex(report_buffer, size);
		} else {
			switch(r) {
			case LIBUSB_ERROR_NOT_FOUND:
				printf("   No Feature Report available for this device\n");
				break;
			case LIBUSB_ERROR_PIPE:
				printf("   Detected stall - resetting pipe...\n");
				libusb_clear_halt(handle, 0);
				break;
			default:
				printf("   Error: %s\n", libusb_strerror((enum libusb_error)r));
				break;
			}
		}
		free(report_buffer);
	}

	size = get_hid_record_size(hid_report_descriptor, descriptor_size, HID_REPORT_TYPE_INPUT);
	if (size <= 0) {
		printf("\nSkipping Input Report readout (None detected)\n");
	} else {
		report_buffer = (uint8_t*) calloc(size, 1);
		if (report_buffer == NULL) {
			return -1;
		}

		printf("\nReading Input Report (length %d)...\n", size);
		r = libusb_control_transfer(handle, LIBUSB_ENDPOINT_IN|LIBUSB_REQUEST_TYPE_CLASS|LIBUSB_RECIPIENT_INTERFACE,
			HID_GET_REPORT, (HID_REPORT_TYPE_INPUT<<8)|0x00, 0, report_buffer, (uint16_t)size, 5000);
		if (r >= 0) {
			display_buffer_hex(report_buffer, size);
		} else {
			switch(r) {
			case LIBUSB_ERROR_TIMEOUT:
				printf("   Timeout! Please make sure you act on the device within the 5 seconds allocated...\n");
				break;
			case LIBUSB_ERROR_PIPE:
				printf("   Detected stall - resetting pipe...\n");
				libusb_clear_halt(handle, 0);
				break;
			default:
				printf("   Error: %s\n", libusb_strerror((enum libusb_error)r));
				break;
			}
		}

		// Attempt a bulk read from endpoint 0 (this should just return a raw input report)
		printf("\nTesting interrupt read using endpoint %02X...\n", endpoint_in);
		r = libusb_interrupt_transfer(handle, endpoint_in, report_buffer, size, &size, 5000);
		if (r >= 0) {
			display_buffer_hex(report_buffer, size);
		} else {
			printf("   %s\n", libusb_strerror((enum libusb_error)r));
		}

		free(report_buffer);
	}
	return 0;
}

// Read the MS WinUSB Feature Descriptors, that are used on Windows 8 for automated driver installation
static void read_ms_winsub_feature_descriptors(libusb_device_handle *handle, uint8_t bRequest, int iface_number)
{
#define MAX_OS_FD_LENGTH 256
	int i, r;
	uint8_t os_desc[MAX_OS_FD_LENGTH];
	uint32_t length;
	void* le_type_punning_IS_fine;
	struct {
		const char* desc;
		uint8_t recipient;
		uint16_t index;
		uint16_t header_size;
	} os_fd[2] = {
		{"Extended Compat ID", LIBUSB_RECIPIENT_DEVICE, 0x0004, 0x10},
		{"Extended Properties", LIBUSB_RECIPIENT_INTERFACE, 0x0005, 0x0A}
	};

	if (iface_number < 0) return;
	// WinUSB has a limitation that forces wIndex to the interface number when issuing
	// an Interface Request. To work around that, we can force a Device Request for
	// the Extended Properties, assuming the device answers both equally.
	if (force_device_request)
		os_fd[1].recipient = LIBUSB_RECIPIENT_DEVICE;

	for (i=0; i<2; i++) {
		printf("\nReading %s OS Feature Descriptor (wIndex = 0x%04d):\n", os_fd[i].desc, os_fd[i].index);

		// Read the header part
		r = libusb_control_transfer(handle, (uint8_t)(LIBUSB_ENDPOINT_IN|LIBUSB_REQUEST_TYPE_VENDOR|os_fd[i].recipient),
			bRequest, (uint16_t)(((iface_number)<< 8)|0x00), os_fd[i].index, os_desc, os_fd[i].header_size, 1000);
		if (r < os_fd[i].header_size) {
			perr("   Failed: %s", (r<0)?libusb_strerror((enum libusb_error)r):"header size is too small");
			return;
		}
		le_type_punning_IS_fine = (void*)os_desc;
		length = *((uint32_t*)le_type_punning_IS_fine);
		if (length > MAX_OS_FD_LENGTH) {
			length = MAX_OS_FD_LENGTH;
		}

		// Read the full feature descriptor
		r = libusb_control_transfer(handle, (uint8_t)(LIBUSB_ENDPOINT_IN|LIBUSB_REQUEST_TYPE_VENDOR|os_fd[i].recipient),
			bRequest, (uint16_t)(((iface_number)<< 8)|0x00), os_fd[i].index, os_desc, (uint16_t)length, 1000);
		if (r < 0) {
			perr("   Failed: %s", libusb_strerror((enum libusb_error)r));
			return;
		} else {
			display_buffer_hex(os_desc, r);
		}
	}
}

static void print_device_cap(struct libusb_bos_dev_capability_descriptor *dev_cap)
{
	switch(dev_cap->bDevCapabilityType) {
	case LIBUSB_BT_USB_2_0_EXTENSION: {
		struct libusb_usb_2_0_extension_descriptor *usb_2_0_ext = NULL;
		libusb_get_usb_2_0_extension_descriptor(NULL, dev_cap, &usb_2_0_ext);
		if (usb_2_0_ext) {
			printf("    USB 2.0 extension:\n");
			printf("      attributes             : %02X\n", usb_2_0_ext->bmAttributes);
			libusb_free_usb_2_0_extension_descriptor(usb_2_0_ext);
		}
		break;
	}
	case LIBUSB_BT_SS_USB_DEVICE_CAPABILITY: {
		struct libusb_ss_usb_device_capability_descriptor *ss_usb_device_cap = NULL;
		libusb_get_ss_usb_device_capability_descriptor(NULL, dev_cap, &ss_usb_device_cap);
		if (ss_usb_device_cap) {
			printf("    USB 3.0 capabilities:\n");
			printf("      attributes             : %02X\n", ss_usb_device_cap->bmAttributes);
			printf("      supported speeds       : %04X\n", ss_usb_device_cap->wSpeedSupported);
			printf("      supported functionality: %02X\n", ss_usb_device_cap->bFunctionalitySupport);
			libusb_free_ss_usb_device_capability_descriptor(ss_usb_device_cap);
		}
		break;
	}
	case LIBUSB_BT_CONTAINER_ID: {
		struct libusb_container_id_descriptor *container_id = NULL;
		libusb_get_container_id_descriptor(NULL, dev_cap, &container_id);
		if (container_id) {
			printf("    Container ID:\n      %s\n", uuid_to_string(container_id->ContainerID));
			libusb_free_container_id_descriptor(container_id);
		}
		break;
	}
	default:
		printf("    Unknown BOS device capability %02x:\n", dev_cap->bDevCapabilityType);
	}
}

static int test_device(uint16_t vid, uint16_t pid)
{
	libusb_device_handle *handle;
	libusb_device *dev;
	uint8_t bus, port_path[8];
	struct libusb_bos_descriptor *bos_desc;
	struct libusb_config_descriptor *conf_desc;
	const struct libusb_endpoint_descriptor *endpoint;
	int i, j, k, r;
	int iface, nb_ifaces, first_iface = -1;
	struct libusb_device_descriptor dev_desc;
	const char* const speed_name[6] = { "Unknown", "1.5 Mbit/s (USB LowSpeed)", "12 Mbit/s (USB FullSpeed)",
		"480 Mbit/s (USB HighSpeed)", "5000 Mbit/s (USB SuperSpeed)", "10000 Mbit/s (USB SuperSpeedPlus)" };
	char string[128];
	uint8_t string_index[3];	// indexes of the string descriptors
	uint8_t endpoint_in = 0, endpoint_out = 0;	// default IN and OUT endpoints

	printf("Opening device %04X:%04X...\n", vid, pid);
	handle = libusb_open_device_with_vid_pid(NULL, vid, pid);

	if (handle == NULL) {
		perr("  Failed.\n");
		return -1;
	}

	dev = libusb_get_device(handle);
	bus = libusb_get_bus_number(dev);
	if (extra_info) {
		r = libusb_get_port_numbers(dev, port_path, sizeof(port_path));
		if (r > 0) {
			printf("\nDevice properties:\n");
			printf("        bus number: %d\n", bus);
			printf("         port path: %d", port_path[0]);
			for (i=1; i<r; i++) {
				printf("->%d", port_path[i]);
			}
			printf(" (from root hub)\n");
		}
		r = libusb_get_device_speed(dev);
		if ((r<0) || (r>5)) r=0;
		printf("             speed: %s\n", speed_name[r]);
	}

	printf("\nReading device descriptor:\n");
	CALL_CHECK_CLOSE(libusb_get_device_descriptor(dev, &dev_desc), handle);
	printf("            length: %d\n", dev_desc.bLength);
	printf("      device class: %d\n", dev_desc.bDeviceClass);
	printf("               S/N: %d\n", dev_desc.iSerialNumber);
	printf("           VID:PID: %04X:%04X\n", dev_desc.idVendor, dev_desc.idProduct);
	printf("         bcdDevice: %04X\n", dev_desc.bcdDevice);
	printf("   iMan:iProd:iSer: %d:%d:%d\n", dev_desc.iManufacturer, dev_desc.iProduct, dev_desc.iSerialNumber);
	printf("          nb confs: %d\n", dev_desc.bNumConfigurations);
	// Copy the string descriptors for easier parsing
	string_index[0] = dev_desc.iManufacturer;
	string_index[1] = dev_desc.iProduct;
	string_index[2] = dev_desc.iSerialNumber;

	printf("\nReading BOS descriptor: ");
	if (libusb_get_bos_descriptor(handle, &bos_desc) == LIBUSB_SUCCESS) {
		printf("%d caps\n", bos_desc->bNumDeviceCaps);
		for (i = 0; i < bos_desc->bNumDeviceCaps; i++)
			print_device_cap(bos_desc->dev_capability[i]);
		libusb_free_bos_descriptor(bos_desc);
	} else {
		printf("no descriptor\n");
	}

	printf("\nReading first configuration descriptor:\n");
	CALL_CHECK_CLOSE(libusb_get_config_descriptor(dev, 0, &conf_desc), handle);
	nb_ifaces = conf_desc->bNumInterfaces;
	printf("             nb interfaces: %d\n", nb_ifaces);
	if (nb_ifaces > 0)
		first_iface = conf_desc->usb_interface[0].altsetting[0].bInterfaceNumber;
	for (i=0; i<nb_ifaces; i++) {
		printf("              interface[%d]: id = %d\n", i,
			conf_desc->usb_interface[i].altsetting[0].bInterfaceNumber);
		for (j=0; j<conf_desc->usb_interface[i].num_altsetting; j++) {
			printf("interface[%d].altsetting[%d]: num endpoints = %d\n",
				i, j, conf_desc->usb_interface[i].altsetting[j].bNumEndpoints);
			printf("   Class.SubClass.Protocol: %02X.%02X.%02X\n",
				conf_desc->usb_interface[i].altsetting[j].bInterfaceClass,
				conf_desc->usb_interface[i].altsetting[j].bInterfaceSubClass,
				conf_desc->usb_interface[i].altsetting[j].bInterfaceProtocol);
			if ( (conf_desc->usb_interface[i].altsetting[j].bInterfaceClass == LIBUSB_CLASS_MASS_STORAGE)
			  && ( (conf_desc->usb_interface[i].altsetting[j].bInterfaceSubClass == 0x01)
			  || (conf_desc->usb_interface[i].altsetting[j].bInterfaceSubClass == 0x06) )
			  && (conf_desc->usb_interface[i].altsetting[j].bInterfaceProtocol == 0x50) ) {
				// Mass storage devices that can use basic SCSI commands
				test_mode = USE_SCSI;
			}
			for (k=0; k<conf_desc->usb_interface[i].altsetting[j].bNumEndpoints; k++) {
				struct libusb_ss_endpoint_companion_descriptor *ep_comp = NULL;
				endpoint = &conf_desc->usb_interface[i].altsetting[j].endpoint[k];
				printf("       endpoint[%d].address: %02X\n", k, endpoint->bEndpointAddress);
				// Use the first interrupt or bulk IN/OUT endpoints as default for testing
				if ((endpoint->bmAttributes & LIBUSB_TRANSFER_TYPE_MASK) & (LIBUSB_TRANSFER_TYPE_BULK | LIBUSB_TRANSFER_TYPE_INTERRUPT)) {
					if (endpoint->bEndpointAddress & LIBUSB_ENDPOINT_IN) {
						if (!endpoint_in)
							endpoint_in = endpoint->bEndpointAddress;
					} else {
						if (!endpoint_out)
							endpoint_out = endpoint->bEndpointAddress;
					}
				}
				printf("           max packet size: %04X\n", endpoint->wMaxPacketSize);
				printf("          polling interval: %02X\n", endpoint->bInterval);
				libusb_get_ss_endpoint_companion_descriptor(NULL, endpoint, &ep_comp);
				if (ep_comp) {
					printf("                 max burst: %02X   (USB 3.0)\n", ep_comp->bMaxBurst);
					printf("        bytes per interval: %04X (USB 3.0)\n", ep_comp->wBytesPerInterval);
					libusb_free_ss_endpoint_companion_descriptor(ep_comp);
				}
			}
		}
	}
	libusb_free_config_descriptor(conf_desc);

	libusb_set_auto_detach_kernel_driver(handle, 1);
	for (iface = 0; iface < nb_ifaces; iface++)
	{
		printf("\nClaiming interface %d...\n", iface);
		r = libusb_claim_interface(handle, iface);
		if (r != LIBUSB_SUCCESS) {
			perr("   Failed.\n");
		}
	}

	printf("\nReading string descriptors:\n");
	for (i=0; i<3; i++) {
		if (string_index[i] == 0) {
			continue;
		}
		if (libusb_get_string_descriptor_ascii(handle, string_index[i], (unsigned char*)string, sizeof(string)) > 0) {
			printf("   String (0x%02X): \"%s\"\n", string_index[i], string);
		}
	}
	// Read the OS String Descriptor
	r = libusb_get_string_descriptor(handle, MS_OS_DESC_STRING_INDEX, 0, (unsigned char*)string, MS_OS_DESC_STRING_LENGTH);
	if (r == MS_OS_DESC_STRING_LENGTH && memcmp(ms_os_desc_string, string, sizeof(ms_os_desc_string)) == 0) {
		// If this is a Microsoft OS String Descriptor,
		// attempt to read the WinUSB extended Feature Descriptors
		read_ms_winsub_feature_descriptors(handle, string[MS_OS_DESC_VENDOR_CODE_OFFSET], first_iface);
	}

	switch(test_mode) {
	case USE_PS3:
		CALL_CHECK_CLOSE(display_ps3_status(handle), handle);
		break;
	case USE_XBOX:
		CALL_CHECK_CLOSE(display_xbox_status(handle), handle);
		CALL_CHECK_CLOSE(set_xbox_actuators(handle, 128, 222), handle);
		msleep(2000);
		CALL_CHECK_CLOSE(set_xbox_actuators(handle, 0, 0), handle);
		break;
	case USE_HID:
		test_hid(handle, endpoint_in);
		break;
	case USE_SCSI:
		CALL_CHECK_CLOSE(test_mass_storage(handle, endpoint_in, endpoint_out), handle);
	case USE_GENERIC:
		break;
	}

	printf("\n");
	for (iface = 0; iface<nb_ifaces; iface++) {
		printf("Releasing interface %d...\n", iface);
		libusb_release_interface(handle, iface);
	}

	printf("Closing device...\n");
	libusb_close(handle);

	return 0;
}

int main(int argc, char** argv)
{
	bool show_help = false;
	bool debug_mode = false;
	const struct libusb_version* version;
	int j, r;
	size_t i, arglen;
	unsigned tmp_vid, tmp_pid;
	uint16_t endian_test = 0xBE00;
	char *error_lang = NULL, *old_dbg_str = NULL, str[256];

	// Default to generic, expecting VID:PID
	VID = 0;
	PID = 0;
	test_mode = USE_GENERIC;

	if (((uint8_t*)&endian_test)[0] == 0xBE) {
		printf("Despite their natural superiority for end users, big endian\n"
			"CPUs are not supported with this program, sorry.\n");
		return 0;
	}

	if (argc >= 2) {
		for (j = 1; j<argc; j++) {
			arglen = strlen(argv[j]);
			if ( ((argv[j][0] == '-') || (argv[j][0] == '/'))
			  && (arglen >= 2) ) {
				switch(argv[j][1]) {
				case 'd':
					debug_mode = true;
					break;
				case 'i':
					extra_info = true;
					break;
				case 'w':
					force_device_request = true;
					break;
				case 'b':
					if ((j+1 >= argc) || (argv[j+1][0] == '-') || (argv[j+1][0] == '/')) {
						printf("   Option -b requires a file name\n");
						return 1;
					}
					binary_name = argv[++j];
					binary_dump = true;
					break;
				case 'l':
					if ((j+1 >= argc) || (argv[j+1][0] == '-') || (argv[j+1][0] == '/')) {
						printf("   Option -l requires an ISO 639-1 language parameter\n");
						return 1;
					}
					error_lang = argv[++j];
					break;
				case 'j':
					// OLIMEX ARM-USB-TINY JTAG, 2 channel composite device - 2 interfaces
					if (!VID && !PID) {
						VID = 0x15BA;
						PID = 0x0004;
					}
					break;
				case 'k':
					// Generic 2 GB USB Key (SCSI Transparent/Bulk Only) - 1 interface
					if (!VID && !PID) {
						VID = 0x0204;
						PID = 0x6025;
					}
					break;
				// The following tests will force VID:PID if already provided
				case 'p':
					// Sony PS3 Controller - 1 interface
					VID = 0x054C;
					PID = 0x0268;
					test_mode = USE_PS3;
					break;
				case 's':
					// Microsoft Sidewinder Precision Pro Joystick - 1 HID interface
					VID = 0x045E;
					PID = 0x0008;
					test_mode = USE_HID;
					break;
				case 'x':
					// Microsoft XBox Controller Type S - 1 interface
					VID = 0x045E;
					PID = 0x0289;
					test_mode = USE_XBOX;
					break;
				default:
					show_help = true;
					break;
				}
			} else {
				for (i=0; i<arglen; i++) {
					if (argv[j][i] == ':')
						break;
				}
				if (i != arglen) {
					if (sscanf(argv[j], "%x:%x" , &tmp_vid, &tmp_pid) != 2) {
						printf("   Please specify VID & PID as \"vid:pid\" in hexadecimal format\n");
						return 1;
					}
					VID = (uint16_t)tmp_vid;
					PID = (uint16_t)tmp_pid;
				} else {
					show_help = true;
				}
			}
		}
	}

	if ((show_help) || (argc == 1) || (argc > 7)) {
		printf("usage: %s [-h] [-d] [-i] [-k] [-b file] [-l lang] [-j] [-x] [-s] [-p] [-w] [vid:pid]\n", argv[0]);
		printf("   -h      : display usage\n");
		printf("   -d      : enable debug output\n");
		printf("   -i      : print topology and speed info\n");
		printf("   -j      : test composite FTDI based JTAG device\n");
		printf("   -k      : test Mass Storage device\n");
		printf("   -b file : dump Mass Storage data to file 'file'\n");
		printf("   -p      : test Sony PS3 SixAxis controller\n");
		printf("   -s      : test Microsoft Sidewinder Precision Pro (HID)\n");
		printf("   -x      : test Microsoft XBox Controller Type S\n");
		printf("   -l lang : language to report errors in (ISO 639-1)\n");
		printf("   -w      : force the use of device requests when querying WCID descriptors\n");
		printf("If only the vid:pid is provided, xusb attempts to run the most appropriate test\n");
		return 0;
	}

	// xusb is commonly used as a debug tool, so it's convenient to have debug output during libusb_init(),
	// but since we can't call on libusb_set_option() before libusb_init(), we use the env variable method
	old_dbg_str = getenv("LIBUSB_DEBUG");
	if (debug_mode) {
		if (putenv("LIBUSB_DEBUG=4") != 0)	// LIBUSB_LOG_LEVEL_DEBUG
			printf("Unable to set debug level\n");
	}

	version = libusb_get_version();
	printf("Using libusb v%d.%d.%d.%d\n\n", version->major, version->minor, version->micro, version->nano);
	r = libusb_init(NULL);
	if (r < 0)
		return r;

	// If not set externally, and no debug option was given, use info log level
	if ((old_dbg_str == NULL) && (!debug_mode))
		libusb_set_option(NULL, LIBUSB_OPTION_LOG_LEVEL, LIBUSB_LOG_LEVEL_INFO);
	if (error_lang != NULL) {
		r = libusb_setlocale(error_lang);
		if (r < 0)
			printf("Invalid or unsupported locale '%s': %s\n", error_lang, libusb_strerror((enum libusb_error)r));
	}

	test_device(VID, PID);

	libusb_exit(NULL);

	if (debug_mode) {
		snprintf(str, sizeof(str), "LIBUSB_DEBUG=%s", (old_dbg_str == NULL)?"":old_dbg_str);
		str[sizeof(str) - 1] = 0;	// Windows may not NUL terminate the string
	}

	return 0;
}
