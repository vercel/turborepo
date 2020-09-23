/*
 * libusb example program to manipulate U.are.U 4000B fingerprint scanner.
 * Copyright © 2007 Daniel Drake <dsd@gentoo.org>
 * Copyright © 2016 Nathan Hjelm <hjelmn@mac.com>
 *
 * Basic image capture program only, does not consider the powerup quirks or
 * the fact that image encryption may be enabled. Not expected to work
 * flawlessly all of the time.
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

#include <errno.h>
#include <pthread.h>
#include <semaphore.h>
#include <signal.h>
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>

#include "libusb.h"

#define EP_INTR			(1 | LIBUSB_ENDPOINT_IN)
#define EP_DATA			(2 | LIBUSB_ENDPOINT_IN)
#define CTRL_IN			(LIBUSB_REQUEST_TYPE_VENDOR | LIBUSB_ENDPOINT_IN)
#define CTRL_OUT		(LIBUSB_REQUEST_TYPE_VENDOR | LIBUSB_ENDPOINT_OUT)
#define USB_RQ			0x04
#define INTR_LENGTH		64
#define SEM_NAME                "/org.libusb.example.dpfp_threaded"

enum {
	MODE_INIT = 0x00,
	MODE_AWAIT_FINGER_ON = 0x10,
	MODE_AWAIT_FINGER_OFF = 0x12,
	MODE_CAPTURE = 0x20,
	MODE_SHUT_UP = 0x30,
	MODE_READY = 0x80,
};

static int next_state(void);

enum {
	STATE_AWAIT_MODE_CHANGE_AWAIT_FINGER_ON = 1,
	STATE_AWAIT_IRQ_FINGER_DETECTED,
	STATE_AWAIT_MODE_CHANGE_CAPTURE,
	STATE_AWAIT_IMAGE,
	STATE_AWAIT_MODE_CHANGE_AWAIT_FINGER_OFF,
	STATE_AWAIT_IRQ_FINGER_REMOVED,
};

static int state = 0;
static struct libusb_device_handle *devh = NULL;
static unsigned char imgbuf[0x1b340];
static unsigned char irqbuf[INTR_LENGTH];
static struct libusb_transfer *img_transfer = NULL;
static struct libusb_transfer *irq_transfer = NULL;
static int img_idx = 0;
static volatile sig_atomic_t do_exit = 0;

static pthread_t poll_thread;
static sem_t *exit_sem;

static void request_exit(sig_atomic_t code)
{
	do_exit = code;
	sem_post(exit_sem);
}

static void *poll_thread_main(void *arg)
{
	int r = 0;
	printf("poll thread running\n");

	(void)arg;

	while (!do_exit) {
		struct timeval tv = { 1, 0 };
		r = libusb_handle_events_timeout(NULL, &tv);
		if (r < 0) {
			request_exit(2);
			break;
		}
	}

	printf("poll thread shutting down\n");
	return NULL;
}

static int find_dpfp_device(void)
{
	devh = libusb_open_device_with_vid_pid(NULL, 0x05ba, 0x000a);
	return devh ? 0 : -EIO;
}

static int print_f0_data(void)
{
	unsigned char data[0x10];
	int r;
	unsigned int i;

	r = libusb_control_transfer(devh, CTRL_IN, USB_RQ, 0xf0, 0, data,
		sizeof(data), 0);
	if (r < 0) {
		fprintf(stderr, "F0 error %d\n", r);
		return r;
	}
	if ((unsigned int) r < sizeof(data)) {
		fprintf(stderr, "short read (%d)\n", r);
		return -1;
	}

	printf("F0 data:");
	for (i = 0; i < sizeof(data); i++)
		printf("%02x ", data[i]);
	printf("\n");
	return 0;
}

static int get_hwstat(unsigned char *status)
{
	int r;

	r = libusb_control_transfer(devh, CTRL_IN, USB_RQ, 0x07, 0, status, 1, 0);
	if (r < 0) {
		fprintf(stderr, "read hwstat error %d\n", r);
		return r;
	}
	if ((unsigned int) r < 1) {
		fprintf(stderr, "short read (%d)\n", r);
		return -1;
	}

	printf("hwstat reads %02x\n", *status);
	return 0;
}

static int set_hwstat(unsigned char data)
{
	int r;

	printf("set hwstat to %02x\n", data);
	r = libusb_control_transfer(devh, CTRL_OUT, USB_RQ, 0x07, 0, &data, 1, 0);
	if (r < 0) {
		fprintf(stderr, "set hwstat error %d\n", r);
		return r;
	}
	if ((unsigned int) r < 1) {
		fprintf(stderr, "short write (%d)", r);
		return -1;
	}

	return 0;
}

static int set_mode(unsigned char data)
{
	int r;
	printf("set mode %02x\n", data);

	r = libusb_control_transfer(devh, CTRL_OUT, USB_RQ, 0x4e, 0, &data, 1, 0);
	if (r < 0) {
		fprintf(stderr, "set mode error %d\n", r);
		return r;
	}
	if ((unsigned int) r < 1) {
		fprintf(stderr, "short write (%d)", r);
		return -1;
	}

	return 0;
}

static void LIBUSB_CALL cb_mode_changed(struct libusb_transfer *transfer)
{
	if (transfer->status != LIBUSB_TRANSFER_COMPLETED) {
		fprintf(stderr, "mode change transfer not completed!\n");
		request_exit(2);
	}

	printf("async cb_mode_changed length=%d actual_length=%d\n",
		transfer->length, transfer->actual_length);
	if (next_state() < 0)
		request_exit(2);
}

static int set_mode_async(unsigned char data)
{
	unsigned char *buf = (unsigned char*) malloc(LIBUSB_CONTROL_SETUP_SIZE + 1);
	struct libusb_transfer *transfer;

	if (!buf)
		return -ENOMEM;

	transfer = libusb_alloc_transfer(0);
	if (!transfer) {
		free(buf);
		return -ENOMEM;
	}

	printf("async set mode %02x\n", data);
	libusb_fill_control_setup(buf, CTRL_OUT, USB_RQ, 0x4e, 0, 1);
	buf[LIBUSB_CONTROL_SETUP_SIZE] = data;
	libusb_fill_control_transfer(transfer, devh, buf, cb_mode_changed, NULL,
		1000);

	transfer->flags = LIBUSB_TRANSFER_SHORT_NOT_OK
		| LIBUSB_TRANSFER_FREE_BUFFER | LIBUSB_TRANSFER_FREE_TRANSFER;
	return libusb_submit_transfer(transfer);
}

static int do_sync_intr(unsigned char *data)
{
	int r;
	int transferred;

	r = libusb_interrupt_transfer(devh, EP_INTR, data, INTR_LENGTH,
		&transferred, 1000);
	if (r < 0) {
		fprintf(stderr, "intr error %d\n", r);
		return r;
	}
	if (transferred < INTR_LENGTH) {
		fprintf(stderr, "short read (%d)\n", r);
		return -1;
	}

	printf("recv interrupt %04x\n", *((uint16_t *) data));
	return 0;
}

static int sync_intr(unsigned char type)
{
	int r;
	unsigned char data[INTR_LENGTH];

	while (1) {
		r = do_sync_intr(data);
		if (r < 0)
			return r;
		if (data[0] == type)
			return 0;
	}
}

static int save_to_file(unsigned char *data)
{
	FILE *fd;
	char filename[64];

	snprintf(filename, sizeof(filename), "finger%d.pgm", img_idx++);
	fd = fopen(filename, "w");
	if (!fd)
		return -1;

	fputs("P5 384 289 255 ", fd);
	(void) fwrite(data + 64, 1, 384*289, fd);
	fclose(fd);
	printf("saved image to %s\n", filename);
	return 0;
}

static int next_state(void)
{
	int r = 0;
	printf("old state: %d\n", state);
	switch (state) {
	case STATE_AWAIT_IRQ_FINGER_REMOVED:
		state = STATE_AWAIT_MODE_CHANGE_AWAIT_FINGER_ON;
		r = set_mode_async(MODE_AWAIT_FINGER_ON);
		break;
	case STATE_AWAIT_MODE_CHANGE_AWAIT_FINGER_ON:
		state = STATE_AWAIT_IRQ_FINGER_DETECTED;
		break;
	case STATE_AWAIT_IRQ_FINGER_DETECTED:
		state = STATE_AWAIT_MODE_CHANGE_CAPTURE;
		r = set_mode_async(MODE_CAPTURE);
		break;
	case STATE_AWAIT_MODE_CHANGE_CAPTURE:
		state = STATE_AWAIT_IMAGE;
		break;
	case STATE_AWAIT_IMAGE:
		state = STATE_AWAIT_MODE_CHANGE_AWAIT_FINGER_OFF;
		r = set_mode_async(MODE_AWAIT_FINGER_OFF);
		break;
	case STATE_AWAIT_MODE_CHANGE_AWAIT_FINGER_OFF:
		state = STATE_AWAIT_IRQ_FINGER_REMOVED;
		break;
	default:
		printf("unrecognised state %d\n", state);
	}
	if (r < 0) {
		fprintf(stderr, "error detected changing state\n");
		return r;
	}

	printf("new state: %d\n", state);
	return 0;
}

static void LIBUSB_CALL cb_irq(struct libusb_transfer *transfer)
{
	unsigned char irqtype = transfer->buffer[0];

	if (transfer->status != LIBUSB_TRANSFER_COMPLETED) {
		fprintf(stderr, "irq transfer status %d?\n", transfer->status);
		irq_transfer = NULL;
		request_exit(2);
		return;
	}

	printf("IRQ callback %02x\n", irqtype);
	switch (state) {
	case STATE_AWAIT_IRQ_FINGER_DETECTED:
		if (irqtype == 0x01) {
			if (next_state() < 0) {
				request_exit(2);
				return;
			}
		} else {
			printf("finger-on-sensor detected in wrong state!\n");
		}
		break;
	case STATE_AWAIT_IRQ_FINGER_REMOVED:
		if (irqtype == 0x02) {
			if (next_state() < 0) {
				request_exit(2);
				return;
			}
		} else {
			printf("finger-on-sensor detected in wrong state!\n");
		}
		break;
	}
	if (libusb_submit_transfer(irq_transfer) < 0)
		request_exit(2);
}

static void LIBUSB_CALL cb_img(struct libusb_transfer *transfer)
{
	if (transfer->status != LIBUSB_TRANSFER_COMPLETED) {
		fprintf(stderr, "img transfer status %d?\n", transfer->status);
		img_transfer = NULL;
		request_exit(2);
		return;
	}

	printf("Image callback\n");
	save_to_file(imgbuf);
	if (next_state() < 0) {
		request_exit(2);
		return;
	}
	if (libusb_submit_transfer(img_transfer) < 0)
		request_exit(2);
}

static int init_capture(void)
{
	int r;

	r = libusb_submit_transfer(irq_transfer);
	if (r < 0)
		return r;

	r = libusb_submit_transfer(img_transfer);
	if (r < 0) {
		libusb_cancel_transfer(irq_transfer);
		while (irq_transfer)
			if (libusb_handle_events(NULL) < 0)
				break;
		return r;
	}

	/* start state machine */
	state = STATE_AWAIT_IRQ_FINGER_REMOVED;
	return next_state();
}

static int do_init(void)
{
	unsigned char status;
	int r;

	r = get_hwstat(&status);
	if (r < 0)
		return r;

	if (!(status & 0x80)) {
		r = set_hwstat(status | 0x80);
		if (r < 0)
			return r;
		r = get_hwstat(&status);
		if (r < 0)
			return r;
	}

	status &= ~0x80;
	r = set_hwstat(status);
	if (r < 0)
		return r;

	r = get_hwstat(&status);
	if (r < 0)
		return r;

	r = sync_intr(0x56);
	if (r < 0)
		return r;

	return 0;
}

static int alloc_transfers(void)
{
	img_transfer = libusb_alloc_transfer(0);
	if (!img_transfer)
		return -ENOMEM;

	irq_transfer = libusb_alloc_transfer(0);
	if (!irq_transfer)
		return -ENOMEM;

	libusb_fill_bulk_transfer(img_transfer, devh, EP_DATA, imgbuf,
		sizeof(imgbuf), cb_img, NULL, 0);
	libusb_fill_interrupt_transfer(irq_transfer, devh, EP_INTR, irqbuf,
		sizeof(irqbuf), cb_irq, NULL, 0);

	return 0;
}

static void sighandler(int signum)
{
	(void)signum;

	request_exit(1);
}

int main(void)
{
	struct sigaction sigact;
	int r = 1;

	exit_sem = sem_open (SEM_NAME, O_CREAT, 0);
	if (!exit_sem) {
		fprintf(stderr, "failed to initialise semaphore error %d", errno);
		exit(1);
	}

	/* only using this semaphore in this process so go ahead and unlink it now */
	sem_unlink (SEM_NAME);

	r = libusb_init(NULL);
	if (r < 0) {
		fprintf(stderr, "failed to initialise libusb\n");
		exit(1);
	}

	r = find_dpfp_device();
	if (r < 0) {
		fprintf(stderr, "Could not find/open device\n");
		goto out;
	}

	r = libusb_claim_interface(devh, 0);
	if (r < 0) {
		fprintf(stderr, "usb_claim_interface error %d %s\n", r, strerror(-r));
		goto out;
	}
	printf("claimed interface\n");

	r = print_f0_data();
	if (r < 0)
		goto out_release;

	r = do_init();
	if (r < 0)
		goto out_deinit;

	/* async from here onwards */

	sigact.sa_handler = sighandler;
	sigemptyset(&sigact.sa_mask);
	sigact.sa_flags = 0;
	sigaction(SIGINT, &sigact, NULL);
	sigaction(SIGTERM, &sigact, NULL);
	sigaction(SIGQUIT, &sigact, NULL);

	r = pthread_create(&poll_thread, NULL, poll_thread_main, NULL);
	if (r)
		goto out_deinit;

	r = alloc_transfers();
	if (r < 0) {
		request_exit(1);
		pthread_join(poll_thread, NULL);
		goto out_deinit;
	}

	r = init_capture();
	if (r < 0) {
		request_exit(1);
		pthread_join(poll_thread, NULL);
		goto out_deinit;
	}

	while (!do_exit)
		sem_wait(exit_sem);

	printf("shutting down...\n");
	pthread_join(poll_thread, NULL);

	r = libusb_cancel_transfer(irq_transfer);
	if (r < 0) {
		request_exit(1);
		goto out_deinit;
	}

	r = libusb_cancel_transfer(img_transfer);
	if (r < 0) {
		request_exit(1);
		goto out_deinit;
	}

	while (img_transfer || irq_transfer)
		if (libusb_handle_events(NULL) < 0)
			break;

	if (do_exit == 1)
		r = 0;
	else
		r = 1;

out_deinit:
	libusb_free_transfer(img_transfer);
	libusb_free_transfer(irq_transfer);
	set_mode(0);
	set_hwstat(0x80);
out_release:
	libusb_release_interface(devh, 0);
out:
	libusb_close(devh);
	libusb_exit(NULL);
	return r >= 0 ? r : -r;
}
