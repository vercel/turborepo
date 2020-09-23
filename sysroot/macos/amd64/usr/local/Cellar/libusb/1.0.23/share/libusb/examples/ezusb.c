/*
 * Copyright © 2001 Stephen Williams (steve@icarus.com)
 * Copyright © 2001-2002 David Brownell (dbrownell@users.sourceforge.net)
 * Copyright © 2008 Roger Williams (rawqux@users.sourceforge.net)
 * Copyright © 2012 Pete Batard (pete@akeo.ie)
 * Copyright © 2013 Federico Manzan (f.manzan@gmail.com)
 *
 *    This source code is free software; you can redistribute it
 *    and/or modify it in source code form under the terms of the GNU
 *    General Public License as published by the Free Software
 *    Foundation; either version 2 of the License, or (at your option)
 *    any later version.
 *
 *    This program is distributed in the hope that it will be useful,
 *    but WITHOUT ANY WARRANTY; without even the implied warranty of
 *    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 *    GNU General Public License for more details.
 *
 *    You should have received a copy of the GNU General Public License
 *    along with this program; if not, write to the Free Software
 *    Foundation, Inc., 59 Temple Place - Suite 330, Boston, MA 02111-1307, USA
 */
#include <stdio.h>
#include <errno.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#include "libusb.h"
#include "ezusb.h"

extern void logerror(const char *format, ...)
	__attribute__ ((format(printf, 1, 2)));

/*
 * This file contains functions for uploading firmware into Cypress
 * EZ-USB microcontrollers. These chips use control endpoint 0 and vendor
 * specific commands to support writing into the on-chip SRAM. They also
 * support writing into the CPUCS register, which is how we reset the
 * processor after loading firmware (including the reset vector).
 *
 * These Cypress devices are 8-bit 8051 based microcontrollers with
 * special support for USB I/O.  They come in several packages, and
 * some can be set up with external memory when device costs allow.
 * Note that the design was originally by AnchorChips, so you may find
 * references to that vendor (which was later merged into Cypress).
 * The Cypress FX parts are largely compatible with the Anchorhip ones.
 */

int verbose = 1;

/*
 * return true if [addr,addr+len] includes external RAM
 * for Anchorchips EZ-USB or Cypress EZ-USB FX
 */
static bool fx_is_external(uint32_t addr, size_t len)
{
	/* with 8KB RAM, 0x0000-0x1b3f can be written
	 * we can't tell if it's a 4KB device here
	 */
	if (addr <= 0x1b3f)
		return ((addr + len) > 0x1b40);

	/* there may be more RAM; unclear if we can write it.
	 * some bulk buffers may be unused, 0x1b3f-0x1f3f
	 * firmware can set ISODISAB for 2KB at 0x2000-0x27ff
	 */
	return true;
}

/*
 * return true if [addr,addr+len] includes external RAM
 * for Cypress EZ-USB FX2
 */
static bool fx2_is_external(uint32_t addr, size_t len)
{
	/* 1st 8KB for data/code, 0x0000-0x1fff */
	if (addr <= 0x1fff)
		return ((addr + len) > 0x2000);

	/* and 512 for data, 0xe000-0xe1ff */
	else if (addr >= 0xe000 && addr <= 0xe1ff)
		return ((addr + len) > 0xe200);

	/* otherwise, it's certainly external */
	else
		return true;
}

/*
 * return true if [addr,addr+len] includes external RAM
 * for Cypress EZ-USB FX2LP
 */
static bool fx2lp_is_external(uint32_t addr, size_t len)
{
	/* 1st 16KB for data/code, 0x0000-0x3fff */
	if (addr <= 0x3fff)
		return ((addr + len) > 0x4000);

	/* and 512 for data, 0xe000-0xe1ff */
	else if (addr >= 0xe000 && addr <= 0xe1ff)
		return ((addr + len) > 0xe200);

	/* otherwise, it's certainly external */
	else
		return true;
}


/*****************************************************************************/

/*
 * These are the requests (bRequest) that the bootstrap loader is expected
 * to recognize.  The codes are reserved by Cypress, and these values match
 * what EZ-USB hardware, or "Vend_Ax" firmware (2nd stage loader) uses.
 * Cypress' "a3load" is nice because it supports both FX and FX2, although
 * it doesn't have the EEPROM support (subset of "Vend_Ax").
 */
#define RW_INTERNAL     0xA0	/* hardware implements this one */
#define RW_MEMORY       0xA3

/*
 * Issues the specified vendor-specific write request.
 */
static int ezusb_write(libusb_device_handle *device, const char *label,
	uint8_t opcode, uint32_t addr, const unsigned char *data, size_t len)
{
	int status;

	if (verbose > 1)
		logerror("%s, addr 0x%08x len %4u (0x%04x)\n", label, addr, (unsigned)len, (unsigned)len);
	status = libusb_control_transfer(device,
		LIBUSB_ENDPOINT_OUT | LIBUSB_REQUEST_TYPE_VENDOR | LIBUSB_RECIPIENT_DEVICE,
		opcode, addr & 0xFFFF, addr >> 16,
		(unsigned char*)data, (uint16_t)len, 1000);
	if (status != (signed)len) {
		if (status < 0)
			logerror("%s: %s\n", label, libusb_error_name(status));
		else
			logerror("%s ==> %d\n", label, status);
	}
	return (status < 0) ? -EIO : 0;
}

/*
 * Issues the specified vendor-specific read request.
 */
static int ezusb_read(libusb_device_handle *device, const char *label,
	uint8_t opcode, uint32_t addr, const unsigned char *data, size_t len)
{
	int status;

	if (verbose > 1)
		logerror("%s, addr 0x%08x len %4u (0x%04x)\n", label, addr, (unsigned)len, (unsigned)len);
	status = libusb_control_transfer(device,
		LIBUSB_ENDPOINT_IN | LIBUSB_REQUEST_TYPE_VENDOR | LIBUSB_RECIPIENT_DEVICE,
		opcode, addr & 0xFFFF, addr >> 16,
		(unsigned char*)data, (uint16_t)len, 1000);
	if (status != (signed)len) {
		if (status < 0)
			logerror("%s: %s\n", label, libusb_error_name(status));
		else
			logerror("%s ==> %d\n", label, status);
	}
	return (status < 0) ? -EIO : 0;
}

/*
 * Modifies the CPUCS register to stop or reset the CPU.
 * Returns false on error.
 */
static bool ezusb_cpucs(libusb_device_handle *device, uint32_t addr, bool doRun)
{
	int status;
	uint8_t data = doRun ? 0x00 : 0x01;

	if (verbose)
		logerror("%s\n", data ? "stop CPU" : "reset CPU");
	status = libusb_control_transfer(device,
		LIBUSB_ENDPOINT_OUT | LIBUSB_REQUEST_TYPE_VENDOR | LIBUSB_RECIPIENT_DEVICE,
		RW_INTERNAL, addr & 0xFFFF, addr >> 16,
		&data, 1, 1000);
	if ((status != 1) &&
		/* We may get an I/O error from libusb as the device disappears */
		((!doRun) || (status != LIBUSB_ERROR_IO)))
	{
		const char *mesg = "can't modify CPUCS";
		if (status < 0)
			logerror("%s: %s\n", mesg, libusb_error_name(status));
		else
			logerror("%s\n", mesg);
		return false;
	} else
		return true;
}

/*
 * Send an FX3 jumpt to address command
 * Returns false on error.
 */
static bool ezusb_fx3_jump(libusb_device_handle *device, uint32_t addr)
{
	int status;

	if (verbose)
		logerror("transfer execution to Program Entry at 0x%08x\n", addr);
	status = libusb_control_transfer(device,
		LIBUSB_ENDPOINT_OUT | LIBUSB_REQUEST_TYPE_VENDOR | LIBUSB_RECIPIENT_DEVICE,
		RW_INTERNAL, addr & 0xFFFF, addr >> 16,
		NULL, 0, 1000);
	/* We may get an I/O error from libusb as the device disappears */
	if ((status != 0) && (status != LIBUSB_ERROR_IO))
	{
		const char *mesg = "failed to send jump command";
		if (status < 0)
			logerror("%s: %s\n", mesg, libusb_error_name(status));
		else
			logerror("%s\n", mesg);
		return false;
	} else
		return true;
}

/*****************************************************************************/

/*
 * Parse an Intel HEX image file and invoke the poke() function on the
 * various segments to implement policies such as writing to RAM (with
 * a one or two stage loader setup, depending on the firmware) or to
 * EEPROM (two stages required).
 *
 * image       - the hex image file
 * context     - for use by poke()
 * is_external - if non-null, used to check which segments go into
 *               external memory (writable only by software loader)
 * poke        - called with each memory segment; errors indicated
 *               by returning negative values.
 *
 * Caller is responsible for halting CPU as needed, such as when
 * overwriting a second stage loader.
 */
static int parse_ihex(FILE *image, void *context,
	bool (*is_external)(uint32_t addr, size_t len),
	int (*poke) (void *context, uint32_t addr, bool external,
	const unsigned char *data, size_t len))
{
	unsigned char data[1023];
	uint32_t data_addr = 0;
	size_t data_len = 0;
	int rc;
	int first_line = 1;
	bool external = false;

	/* Read the input file as an IHEX file, and report the memory segments
	 * as we go.  Each line holds a max of 16 bytes, but uploading is
	 * faster (and EEPROM space smaller) if we merge those lines into larger
	 * chunks.  Most hex files keep memory segments together, which makes
	 * such merging all but free.  (But it may still be worth sorting the
	 * hex files to make up for undesirable behavior from tools.)
	 *
	 * Note that EEPROM segments max out at 1023 bytes; the upload protocol
	 * allows segments of up to 64 KBytes (more than a loader could handle).
	 */
	for (;;) {
		char buf[512], *cp;
		char tmp, type;
		size_t len;
		unsigned idx, off;

		cp = fgets(buf, sizeof(buf), image);
		if (cp == NULL) {
			logerror("EOF without EOF record!\n");
			break;
		}

		/* EXTENSION: "# comment-till-end-of-line", for copyrights etc */
		if (buf[0] == '#')
			continue;

		if (buf[0] != ':') {
			logerror("not an ihex record: %s", buf);
			return -2;
		}

		/* ignore any newline */
		cp = strchr(buf, '\n');
		if (cp)
			*cp = 0;

		if (verbose >= 3)
			logerror("** LINE: %s\n", buf);

		/* Read the length field (up to 16 bytes) */
		tmp = buf[3];
		buf[3] = 0;
		len = strtoul(buf+1, NULL, 16);
		buf[3] = tmp;

		/* Read the target offset (address up to 64KB) */
		tmp = buf[7];
		buf[7] = 0;
		off = (unsigned int)strtoul(buf+3, NULL, 16);
		buf[7] = tmp;

		/* Initialize data_addr */
		if (first_line) {
			data_addr = off;
			first_line = 0;
		}

		/* Read the record type */
		tmp = buf[9];
		buf[9] = 0;
		type = (char)strtoul(buf+7, NULL, 16);
		buf[9] = tmp;

		/* If this is an EOF record, then make it so. */
		if (type == 1) {
			if (verbose >= 2)
				logerror("EOF on hexfile\n");
			break;
		}

		if (type != 0) {
			logerror("unsupported record type: %u\n", type);
			return -3;
		}

		if ((len * 2) + 11 > strlen(buf)) {
			logerror("record too short?\n");
			return -4;
		}

		/* FIXME check for _physically_ contiguous not just virtually
		 * e.g. on FX2 0x1f00-0x2100 includes both on-chip and external
		 * memory so it's not really contiguous */

		/* flush the saved data if it's not contiguous,
		* or when we've buffered as much as we can.
		*/
		if (data_len != 0
			&& (off != (data_addr + data_len)
			/* || !merge */
			|| (data_len + len) > sizeof(data))) {
				if (is_external)
					external = is_external(data_addr, data_len);
				rc = poke(context, data_addr, external, data, data_len);
				if (rc < 0)
					return -1;
				data_addr = off;
				data_len = 0;
		}

		/* append to saved data, flush later */
		for (idx = 0, cp = buf+9 ;  idx < len ;  idx += 1, cp += 2) {
			tmp = cp[2];
			cp[2] = 0;
			data[data_len + idx] = (uint8_t)strtoul(cp, NULL, 16);
			cp[2] = tmp;
		}
		data_len += len;
	}


	/* flush any data remaining */
	if (data_len != 0) {
		if (is_external)
			external = is_external(data_addr, data_len);
		rc = poke(context, data_addr, external, data, data_len);
		if (rc < 0)
			return -1;
	}
	return 0;
}

/*
 * Parse a binary image file and write it as is to the target.
 * Applies to Cypress BIX images for RAM or Cypress IIC images
 * for EEPROM.
 *
 * image       - the BIX image file
 * context     - for use by poke()
 * is_external - if non-null, used to check which segments go into
 *               external memory (writable only by software loader)
 * poke        - called with each memory segment; errors indicated
 *               by returning negative values.
 *
 * Caller is responsible for halting CPU as needed, such as when
 * overwriting a second stage loader.
 */
static int parse_bin(FILE *image, void *context,
	bool (*is_external)(uint32_t addr, size_t len), int (*poke)(void *context,
	uint32_t addr, bool external, const unsigned char *data, size_t len))
{
	unsigned char data[4096];
	uint32_t data_addr = 0;
	size_t data_len = 0;
	int rc;
	bool external = false;

	for (;;) {
		data_len = fread(data, 1, 4096, image);
		if (data_len == 0)
			break;
		if (is_external)
			external = is_external(data_addr, data_len);
		rc = poke(context, data_addr, external, data, data_len);
		if (rc < 0)
			return -1;
		data_addr += (uint32_t)data_len;
	}
	return feof(image)?0:-1;
}

/*
 * Parse a Cypress IIC image file and invoke the poke() function on the
 * various segments for writing to RAM
 *
 * image       - the IIC image file
 * context     - for use by poke()
 * is_external - if non-null, used to check which segments go into
 *               external memory (writable only by software loader)
 * poke        - called with each memory segment; errors indicated
 *               by returning negative values.
 *
 * Caller is responsible for halting CPU as needed, such as when
 * overwriting a second stage loader.
 */
static int parse_iic(FILE *image, void *context,
	bool (*is_external)(uint32_t addr, size_t len),
	int (*poke)(void *context, uint32_t addr, bool external, const unsigned char *data, size_t len))
{
	unsigned char data[4096];
	uint32_t data_addr = 0;
	size_t data_len = 0, read_len;
	uint8_t block_header[4];
	int rc;
	bool external = false;
	long file_size, initial_pos;

	initial_pos = ftell(image);
	if (initial_pos < 0)
		return -1;

	if (fseek(image, 0L, SEEK_END) != 0)
		return -1;
	file_size = ftell(image);
	if (fseek(image, initial_pos, SEEK_SET) != 0)
		return -1;
	for (;;) {
		/* Ignore the trailing reset IIC data (5 bytes) */
		if (ftell(image) >= (file_size - 5))
			break;
		if (fread(&block_header, 1, sizeof(block_header), image) != 4) {
			logerror("unable to read IIC block header\n");
			return -1;
		}
		data_len = (block_header[0] << 8) + block_header[1];
		data_addr = (block_header[2] << 8) + block_header[3];
		if (data_len > sizeof(data)) {
			/* If this is ever reported as an error, switch to using malloc/realloc */
			logerror("IIC data block too small - please report this error to libusb.info\n");
			return -1;
		}
		read_len = fread(data, 1, data_len, image);
		if (read_len != data_len) {
			logerror("read error\n");
			return -1;
		}
		if (is_external)
			external = is_external(data_addr, data_len);
		rc = poke(context, data_addr, external, data, data_len);
		if (rc < 0)
			return -1;
	}
	return 0;
}

/* the parse call will be selected according to the image type */
static int (*parse[IMG_TYPE_MAX])(FILE *image, void *context, bool (*is_external)(uint32_t addr, size_t len),
           int (*poke)(void *context, uint32_t addr, bool external, const unsigned char *data, size_t len))
           = { parse_ihex, parse_iic, parse_bin };

/*****************************************************************************/

/*
 * For writing to RAM using a first (hardware) or second (software)
 * stage loader and 0xA0 or 0xA3 vendor requests
 */
typedef enum {
	_undef = 0,
	internal_only,		/* hardware first-stage loader */
	skip_internal,		/* first phase, second-stage loader */
	skip_external		/* second phase, second-stage loader */
} ram_mode;

struct ram_poke_context {
	libusb_device_handle *device;
	ram_mode mode;
	size_t total, count;
};

#define RETRY_LIMIT 5

static int ram_poke(void *context, uint32_t addr, bool external,
	const unsigned char *data, size_t len)
{
	struct ram_poke_context *ctx = (struct ram_poke_context*)context;
	int rc;
	unsigned retry = 0;

	switch (ctx->mode) {
	case internal_only:		/* CPU should be stopped */
		if (external) {
			logerror("can't write %u bytes external memory at 0x%08x\n",
				(unsigned)len, addr);
			return -EINVAL;
		}
		break;
	case skip_internal:		/* CPU must be running */
		if (!external) {
			if (verbose >= 2) {
				logerror("SKIP on-chip RAM, %u bytes at 0x%08x\n",
					(unsigned)len, addr);
			}
			return 0;
		}
		break;
	case skip_external:		/* CPU should be stopped */
		if (external) {
			if (verbose >= 2) {
				logerror("SKIP external RAM, %u bytes at 0x%08x\n",
					(unsigned)len, addr);
			}
			return 0;
		}
		break;
	case _undef:
	default:
		logerror("bug\n");
		return -EDOM;
	}

	ctx->total += len;
	ctx->count++;

	/* Retry this till we get a real error. Control messages are not
	 * NAKed (just dropped) so time out means is a real problem.
	 */
	while ((rc = ezusb_write(ctx->device,
		external ? "write external" : "write on-chip",
		external ? RW_MEMORY : RW_INTERNAL,
		addr, data, len)) < 0
		&& retry < RETRY_LIMIT) {
		if (rc != LIBUSB_ERROR_TIMEOUT)
			break;
		retry += 1;
	}
	return rc;
}

/*
 * Load a Cypress Image file into target RAM.
 * See http://www.cypress.com/?docID=41351 (AN76405 PDF) for more info.
 */
static int fx3_load_ram(libusb_device_handle *device, const char *path)
{
	uint32_t dCheckSum, dExpectedCheckSum, dAddress, i, dLen, dLength;
	uint32_t* dImageBuf;
	unsigned char *bBuf, hBuf[4], blBuf[4], rBuf[4096];
	FILE *image;
	int ret = 0;

	image = fopen(path, "rb");
	if (image == NULL) {
		logerror("unable to open '%s' for input\n", path);
		return -2;
	} else if (verbose)
		logerror("open firmware image %s for RAM upload\n", path);

	// Read header
	if (fread(hBuf, sizeof(char), sizeof(hBuf), image) != sizeof(hBuf)) {
		logerror("could not read image header");
		ret = -3;
		goto exit;
	}

	// check "CY" signature byte and format
	if ((hBuf[0] != 'C') || (hBuf[1] != 'Y')) {
		logerror("image doesn't have a CYpress signature\n");
		ret = -3;
		goto exit;
	}

	// Check bImageType
	switch(hBuf[3]) {
	case 0xB0:
		if (verbose)
			logerror("normal FW binary %s image with checksum\n", (hBuf[2]&0x01)?"data":"executable");
		break;
	case 0xB1:
		logerror("security binary image is not currently supported\n");
		ret = -3;
		goto exit;
	case 0xB2:
		logerror("VID:PID image is not currently supported\n");
		ret = -3;
		goto exit;
	default:
		logerror("invalid image type 0x%02X\n", hBuf[3]);
		ret = -3;
		goto exit;
	}

	// Read the bootloader version
	if (verbose) {
		if ((ezusb_read(device, "read bootloader version", RW_INTERNAL, 0xFFFF0020, blBuf, 4) < 0)) {
			logerror("Could not read bootloader version\n");
			ret = -8;
			goto exit;
		}
		logerror("FX3 bootloader version: 0x%02X%02X%02X%02X\n", blBuf[3], blBuf[2], blBuf[1], blBuf[0]);
	}

	dCheckSum = 0;
	if (verbose)
		logerror("writing image...\n");
	while (1) {
		if ((fread(&dLength, sizeof(uint32_t), 1, image) != 1) ||  // read dLength
			(fread(&dAddress, sizeof(uint32_t), 1, image) != 1)) { // read dAddress
			logerror("could not read image");
			ret = -3;
			goto exit;
		}
		if (dLength == 0)
			break; // done

		// coverity[tainted_data]
		dImageBuf = (uint32_t*)calloc(dLength, sizeof(uint32_t));
		if (dImageBuf == NULL) {
			logerror("could not allocate buffer for image chunk\n");
			ret = -4;
			goto exit;
		}

		// read sections
		if (fread(dImageBuf, sizeof(uint32_t), dLength, image) != dLength) {
			logerror("could not read image");
			free(dImageBuf);
			ret = -3;
			goto exit;
		}
		for (i = 0; i < dLength; i++)
			dCheckSum += dImageBuf[i];
		dLength <<= 2; // convert to Byte length
		bBuf = (unsigned char*) dImageBuf;

		while (dLength > 0) {
			dLen = 4096; // 4K max
			if (dLen > dLength)
				dLen = dLength;
			if ((ezusb_write(device, "write firmware", RW_INTERNAL, dAddress, bBuf, dLen) < 0) ||
				(ezusb_read(device, "read firmware", RW_INTERNAL, dAddress, rBuf, dLen) < 0)) {
				logerror("R/W error\n");
				free(dImageBuf);
				ret = -5;
				goto exit;
			}
			// Verify data: rBuf with bBuf
			for (i = 0; i < dLen; i++) {
				if (rBuf[i] != bBuf[i]) {
					logerror("verify error");
					free(dImageBuf);
					ret = -6;
					goto exit;
				}
			}

			dLength -= dLen;
			bBuf += dLen;
			dAddress += dLen;
		}
		free(dImageBuf);
	}

	// read pre-computed checksum data
	if ((fread(&dExpectedCheckSum, sizeof(uint32_t), 1, image) != 1) ||
		(dCheckSum != dExpectedCheckSum)) {
		logerror("checksum error\n");
		ret = -7;
		goto exit;
	}

	// transfer execution to Program Entry
	if (!ezusb_fx3_jump(device, dAddress)) {
		ret = -6;
	}

exit:
	fclose(image);
	return ret;
}

/*
 * Load a firmware file into target RAM. device is the open libusb
 * device, and the path is the name of the source file. Open the file,
 * parse the bytes, and write them in one or two phases.
 *
 * If stage == 0, this uses the first stage loader, built into EZ-USB
 * hardware but limited to writing on-chip memory or CPUCS.  Everything
 * is written during one stage, unless there's an error such as the image
 * holding data that needs to be written to external memory.
 *
 * Otherwise, things are written in two stages.  First the external
 * memory is written, expecting a second stage loader to have already
 * been loaded.  Then file is re-parsed and on-chip memory is written.
 */
int ezusb_load_ram(libusb_device_handle *device, const char *path, int fx_type, int img_type, int stage)
{
	FILE *image;
	uint32_t cpucs_addr;
	bool (*is_external)(uint32_t off, size_t len);
	struct ram_poke_context ctx;
	int status;
	uint8_t iic_header[8] = { 0 };
	int ret = 0;

	if (fx_type == FX_TYPE_FX3)
		return fx3_load_ram(device, path);

	image = fopen(path, "rb");
	if (image == NULL) {
		logerror("%s: unable to open for input.\n", path);
		return -2;
	} else if (verbose > 1)
		logerror("open firmware image %s for RAM upload\n", path);

	if (img_type == IMG_TYPE_IIC) {
		if ( (fread(iic_header, 1, sizeof(iic_header), image) != sizeof(iic_header))
		  || (((fx_type == FX_TYPE_FX2LP) || (fx_type == FX_TYPE_FX2)) && (iic_header[0] != 0xC2))
		  || ((fx_type == FX_TYPE_AN21) && (iic_header[0] != 0xB2))
		  || ((fx_type == FX_TYPE_FX1) && (iic_header[0] != 0xB6)) ) {
			logerror("IIC image does not contain executable code - cannot load to RAM.\n");
			ret = -1;
			goto exit;
		}
	}

	/* EZ-USB original/FX and FX2 devices differ, apart from the 8051 core */
	switch(fx_type) {
	case FX_TYPE_FX2LP:
		cpucs_addr = 0xe600;
		is_external = fx2lp_is_external;
		break;
	case FX_TYPE_FX2:
		cpucs_addr = 0xe600;
		is_external = fx2_is_external;
		break;
	default:
		cpucs_addr = 0x7f92;
		is_external = fx_is_external;
		break;
	}

	/* use only first stage loader? */
	if (stage == 0) {
		ctx.mode = internal_only;

		/* if required, halt the CPU while we overwrite its code/data */
		if (cpucs_addr && !ezusb_cpucs(device, cpucs_addr, false))
		{
			ret = -1;
			goto exit;
		}

		/* 2nd stage, first part? loader was already uploaded */
	} else {
		ctx.mode = skip_internal;

		/* let CPU run; overwrite the 2nd stage loader later */
		if (verbose)
			logerror("2nd stage: write external memory\n");
	}

	/* scan the image, first (maybe only) time */
	ctx.device = device;
	ctx.total = ctx.count = 0;
	status = parse[img_type](image, &ctx, is_external, ram_poke);
	if (status < 0) {
		logerror("unable to upload %s\n", path);
		ret = status;
		goto exit;
	}

	/* second part of 2nd stage: rescan */
	// TODO: what should we do for non HEX images there?
	if (stage) {
		ctx.mode = skip_external;

		/* if needed, halt the CPU while we overwrite the 1st stage loader */
		if (cpucs_addr && !ezusb_cpucs(device, cpucs_addr, false))
		{
			ret = -1;
			goto exit;
		}

		/* at least write the interrupt vectors (at 0x0000) for reset! */
		rewind(image);
		if (verbose)
			logerror("2nd stage: write on-chip memory\n");
		status = parse_ihex(image, &ctx, is_external, ram_poke);
		if (status < 0) {
			logerror("unable to completely upload %s\n", path);
			ret = status;
			goto exit;
		}
	}

	if (verbose && (ctx.count != 0)) {
		logerror("... WROTE: %d bytes, %d segments, avg %d\n",
			(int)ctx.total, (int)ctx.count, (int)(ctx.total/ctx.count));
	}

	/* if required, reset the CPU so it runs what we just uploaded */
	if (cpucs_addr && !ezusb_cpucs(device, cpucs_addr, true))
		ret = -1;

exit:
	fclose(image);
	return ret;
}
