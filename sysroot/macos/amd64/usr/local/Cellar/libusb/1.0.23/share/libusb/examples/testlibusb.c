/*
* Test suite program based of libusb-0.1-compat testlibusb
* Copyright (c) 2013 Nathan Hjelm <hjelmn@mac.ccom>
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
#include <string.h>
#include "libusb.h"

#if defined(_MSC_VER) && (_MSC_VER < 1900)
#define snprintf _snprintf
#endif

int verbose = 0;

static void print_endpoint_comp(const struct libusb_ss_endpoint_companion_descriptor *ep_comp)
{
	printf("      USB 3.0 Endpoint Companion:\n");
	printf("        bMaxBurst:        %d\n", ep_comp->bMaxBurst);
	printf("        bmAttributes:     0x%02x\n", ep_comp->bmAttributes);
	printf("        wBytesPerInterval: %d\n", ep_comp->wBytesPerInterval);
}

static void print_endpoint(const struct libusb_endpoint_descriptor *endpoint)
{
	int i, ret;

	printf("      Endpoint:\n");
	printf("        bEndpointAddress: %02xh\n", endpoint->bEndpointAddress);
	printf("        bmAttributes:     %02xh\n", endpoint->bmAttributes);
	printf("        wMaxPacketSize:   %d\n", endpoint->wMaxPacketSize);
	printf("        bInterval:        %d\n", endpoint->bInterval);
	printf("        bRefresh:         %d\n", endpoint->bRefresh);
	printf("        bSynchAddress:    %d\n", endpoint->bSynchAddress);

	for (i = 0; i < endpoint->extra_length;) {
		if (LIBUSB_DT_SS_ENDPOINT_COMPANION == endpoint->extra[i + 1]) {
			struct libusb_ss_endpoint_companion_descriptor *ep_comp;

			ret = libusb_get_ss_endpoint_companion_descriptor(NULL, endpoint, &ep_comp);
			if (LIBUSB_SUCCESS != ret) {
				continue;
			}

			print_endpoint_comp(ep_comp);

			libusb_free_ss_endpoint_companion_descriptor(ep_comp);
		}

		i += endpoint->extra[i];
	}
}

static void print_altsetting(const struct libusb_interface_descriptor *interface)
{
	uint8_t i;

	printf("    Interface:\n");
	printf("      bInterfaceNumber:   %d\n", interface->bInterfaceNumber);
	printf("      bAlternateSetting:  %d\n", interface->bAlternateSetting);
	printf("      bNumEndpoints:      %d\n", interface->bNumEndpoints);
	printf("      bInterfaceClass:    %d\n", interface->bInterfaceClass);
	printf("      bInterfaceSubClass: %d\n", interface->bInterfaceSubClass);
	printf("      bInterfaceProtocol: %d\n", interface->bInterfaceProtocol);
	printf("      iInterface:         %d\n", interface->iInterface);

	for (i = 0; i < interface->bNumEndpoints; i++)
		print_endpoint(&interface->endpoint[i]);
}

static void print_2_0_ext_cap(struct libusb_usb_2_0_extension_descriptor *usb_2_0_ext_cap)
{
	printf("    USB 2.0 Extension Capabilities:\n");
	printf("      bDevCapabilityType: %d\n", usb_2_0_ext_cap->bDevCapabilityType);
	printf("      bmAttributes:       0x%x\n", usb_2_0_ext_cap->bmAttributes);
}

static void print_ss_usb_cap(struct libusb_ss_usb_device_capability_descriptor *ss_usb_cap)
{
	printf("    USB 3.0 Capabilities:\n");
	printf("      bDevCapabilityType: %d\n", ss_usb_cap->bDevCapabilityType);
	printf("      bmAttributes:       0x%x\n", ss_usb_cap->bmAttributes);
	printf("      wSpeedSupported:    0x%x\n", ss_usb_cap->wSpeedSupported);
	printf("      bFunctionalitySupport: %d\n", ss_usb_cap->bFunctionalitySupport);
	printf("      bU1devExitLat:      %d\n", ss_usb_cap->bU1DevExitLat);
	printf("      bU2devExitLat:      %d\n", ss_usb_cap->bU2DevExitLat);
}

static void print_bos(libusb_device_handle *handle)
{
	struct libusb_bos_descriptor *bos;
	int ret;

	ret = libusb_get_bos_descriptor(handle, &bos);
	if (0 > ret) {
		return;
	}

	printf("  Binary Object Store (BOS):\n");
	printf("    wTotalLength:       %d\n", bos->wTotalLength);
	printf("    bNumDeviceCaps:     %d\n", bos->bNumDeviceCaps);

	if(bos->dev_capability[0]->bDevCapabilityType == LIBUSB_BT_USB_2_0_EXTENSION) {

		struct libusb_usb_2_0_extension_descriptor *usb_2_0_extension;
	        ret =  libusb_get_usb_2_0_extension_descriptor(NULL, bos->dev_capability[0],&usb_2_0_extension);
	        if (0 > ret) {
		        return;
	        }

                print_2_0_ext_cap(usb_2_0_extension);
                libusb_free_usb_2_0_extension_descriptor(usb_2_0_extension);
        }

	if(bos->dev_capability[0]->bDevCapabilityType == LIBUSB_BT_SS_USB_DEVICE_CAPABILITY) {

	        struct libusb_ss_usb_device_capability_descriptor *dev_cap;
		ret = libusb_get_ss_usb_device_capability_descriptor(NULL, bos->dev_capability[0],&dev_cap);
	        if (0 > ret) {
		        return;
	        }

	        print_ss_usb_cap(dev_cap);
	        libusb_free_ss_usb_device_capability_descriptor(dev_cap);
        }

	libusb_free_bos_descriptor(bos);
}

static void print_interface(const struct libusb_interface *interface)
{
	int i;

	for (i = 0; i < interface->num_altsetting; i++)
		print_altsetting(&interface->altsetting[i]);
}

static void print_configuration(struct libusb_config_descriptor *config)
{
	uint8_t i;

	printf("  Configuration:\n");
	printf("    wTotalLength:         %d\n", config->wTotalLength);
	printf("    bNumInterfaces:       %d\n", config->bNumInterfaces);
	printf("    bConfigurationValue:  %d\n", config->bConfigurationValue);
	printf("    iConfiguration:       %d\n", config->iConfiguration);
	printf("    bmAttributes:         %02xh\n", config->bmAttributes);
	printf("    MaxPower:             %d\n", config->MaxPower);

	for (i = 0; i < config->bNumInterfaces; i++)
		print_interface(&config->interface[i]);
}

static int print_device(libusb_device *dev, int level)
{
	struct libusb_device_descriptor desc;
	libusb_device_handle *handle = NULL;
	char description[260];
	char string[256];
	int ret;
	uint8_t i;

	ret = libusb_get_device_descriptor(dev, &desc);
	if (ret < 0) {
		fprintf(stderr, "failed to get device descriptor");
		return -1;
	}

	ret = libusb_open(dev, &handle);
	if (LIBUSB_SUCCESS == ret) {
		if (desc.iManufacturer) {
			ret = libusb_get_string_descriptor_ascii(handle, desc.iManufacturer, string, sizeof(string));
			if (ret > 0)
				snprintf(description, sizeof(description), "%s - ", string);
			else
				snprintf(description, sizeof(description), "%04X - ",
				desc.idVendor);
		}
		else
			snprintf(description, sizeof(description), "%04X - ",
			desc.idVendor);

		if (desc.iProduct) {
			ret = libusb_get_string_descriptor_ascii(handle, desc.iProduct, string, sizeof(string));
			if (ret > 0)
				snprintf(description + strlen(description), sizeof(description) -
				strlen(description), "%s", string);
			else
				snprintf(description + strlen(description), sizeof(description) -
				strlen(description), "%04X", desc.idProduct);
		}
		else
			snprintf(description + strlen(description), sizeof(description) -
			strlen(description), "%04X", desc.idProduct);
	}
	else {
		snprintf(description, sizeof(description), "%04X - %04X",
			desc.idVendor, desc.idProduct);
	}

	printf("%.*sDev (bus %d, device %d): %s\n", level * 2, "                    ",
		libusb_get_bus_number(dev), libusb_get_device_address(dev), description);

	if (handle && verbose) {
		if (desc.iSerialNumber) {
			ret = libusb_get_string_descriptor_ascii(handle, desc.iSerialNumber, string, sizeof(string));
			if (ret > 0)
				printf("%.*s  - Serial Number: %s\n", level * 2,
				"                    ", string);
		}
	}

	if (verbose) {
		for (i = 0; i < desc.bNumConfigurations; i++) {
			struct libusb_config_descriptor *config;
			ret = libusb_get_config_descriptor(dev, i, &config);
			if (LIBUSB_SUCCESS != ret) {
				printf("  Couldn't retrieve descriptors\n");
				continue;
			}

			print_configuration(config);

			libusb_free_config_descriptor(config);
		}

		if (handle && desc.bcdUSB >= 0x0201) {
			print_bos(handle);
		}
	}

	if (handle)
		libusb_close(handle);

	return 0;
}

int main(int argc, char *argv[])
{
	libusb_device **devs;
	ssize_t cnt;
	int r, i;

	if (argc > 1 && !strcmp(argv[1], "-v"))
		verbose = 1;

	r = libusb_init(NULL);
	if (r < 0)
		return r;

	cnt = libusb_get_device_list(NULL, &devs);
	if (cnt < 0)
		return (int)cnt;

	for (i = 0; devs[i]; ++i) {
		print_device(devs[i], 0);
	}

	libusb_free_device_list(devs, 1);

	libusb_exit(NULL);
	return 0;
}
