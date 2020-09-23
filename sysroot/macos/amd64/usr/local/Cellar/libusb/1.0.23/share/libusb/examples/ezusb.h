#ifndef ezusb_H
#define ezusb_H
/*
 * Copyright © 2001 Stephen Williams (steve@icarus.com)
 * Copyright © 2002 David Brownell (dbrownell@users.sourceforge.net)
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
#if !defined(_MSC_VER)
#include <stdbool.h>
#else
#define __attribute__(x)
#if !defined(bool)
#define bool int
#endif
#if !defined(true)
#define true (1 == 1)
#endif
#if !defined(false)
#define false (!true)
#endif
#if defined(_PREFAST_)
#pragma warning(disable:28193)
#endif
#endif

#define FX_TYPE_UNDEFINED  -1
#define FX_TYPE_AN21       0	/* Original AnchorChips parts */
#define FX_TYPE_FX1        1	/* Updated Cypress versions */
#define FX_TYPE_FX2        2	/* USB 2.0 versions */
#define FX_TYPE_FX2LP      3	/* Updated FX2 */
#define FX_TYPE_FX3        4	/* USB 3.0 versions */
#define FX_TYPE_MAX        5
#define FX_TYPE_NAMES      { "an21", "fx", "fx2", "fx2lp", "fx3" }

#define IMG_TYPE_UNDEFINED -1
#define IMG_TYPE_HEX       0	/* Intel HEX */
#define IMG_TYPE_IIC       1	/* Cypress 8051 IIC */
#define IMG_TYPE_BIX       2	/* Cypress 8051 BIX */
#define IMG_TYPE_IMG       3	/* Cypress IMG format */
#define IMG_TYPE_MAX       4
#define IMG_TYPE_NAMES     { "Intel HEX", "Cypress 8051 IIC", "Cypress 8051 BIX", "Cypress IMG format" }

#ifdef __cplusplus
extern "C" {
#endif

/* 
 * Automatically identified devices (VID, PID, type, designation).
 * TODO: Could use some validation. Also where's the FX2?
 */
typedef struct {
	uint16_t vid;
	uint16_t pid;
	int type;
	const char* designation;
} fx_known_device;

#define FX_KNOWN_DEVICES { \
	{ 0x0547, 0x2122, FX_TYPE_AN21, "Cypress EZ-USB (2122S)" },\
	{ 0x0547, 0x2125, FX_TYPE_AN21, "Cypress EZ-USB (2121S/2125S)" },\
	{ 0x0547, 0x2126, FX_TYPE_AN21, "Cypress EZ-USB (2126S)" },\
	{ 0x0547, 0x2131, FX_TYPE_AN21, "Cypress EZ-USB (2131Q/2131S/2135S)" },\
	{ 0x0547, 0x2136, FX_TYPE_AN21, "Cypress EZ-USB (2136S)" },\
	{ 0x0547, 0x2225, FX_TYPE_AN21, "Cypress EZ-USB (2225)" },\
	{ 0x0547, 0x2226, FX_TYPE_AN21, "Cypress EZ-USB (2226)" },\
	{ 0x0547, 0x2235, FX_TYPE_AN21, "Cypress EZ-USB (2235)" },\
	{ 0x0547, 0x2236, FX_TYPE_AN21, "Cypress EZ-USB (2236)" },\
	{ 0x04b4, 0x6473, FX_TYPE_FX1, "Cypress EZ-USB FX1" },\
	{ 0x04b4, 0x8613, FX_TYPE_FX2LP, "Cypress EZ-USB FX2LP (68013A/68014A/68015A/68016A)" }, \
	{ 0x04b4, 0x00f3, FX_TYPE_FX3, "Cypress FX3" },\
}

/*
 * This function uploads the firmware from the given file into RAM.
 * Stage == 0 means this is a single stage load (or the first of
 * two stages).  Otherwise it's the second of two stages; the 
 * caller having preloaded the second stage loader.
 *
 * The target processor is reset at the end of this upload.
 */
extern int ezusb_load_ram(libusb_device_handle *device,
	const char *path, int fx_type, int img_type, int stage);

/*
 * This function uploads the firmware from the given file into EEPROM.
 * This uses the right CPUCS address to terminate the EEPROM load with
 * a reset command where FX parts behave differently than FX2 ones.
 * The configuration byte is as provided here (zero for an21xx parts)
 * and the EEPROM type is set so that the microcontroller will boot
 * from it.
 *
 * The caller must have preloaded a second stage loader that knows
 * how to respond to the EEPROM write request.
 */
extern int ezusb_load_eeprom(libusb_device_handle *device,
	const char *path, int fx_type, int img_type, int config);

/* Verbosity level (default 1). Can be increased or decreased with options v/q  */
extern int verbose;

#ifdef __cplusplus
}
#endif

#endif
