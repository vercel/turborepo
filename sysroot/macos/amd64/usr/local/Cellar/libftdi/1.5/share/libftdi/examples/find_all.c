/* find_all.c

   Example for ftdi_usb_find_all()

   This program is distributed under the GPL, version 2
*/

#include <stdio.h>
#include <stdlib.h>
#include <ftdi.h>

int main(void)
{
    int ret, i;
    struct ftdi_context *ftdi;
    struct ftdi_device_list *devlist, *curdev;
    char manufacturer[128], description[128];
    int retval = EXIT_SUCCESS;

    if ((ftdi = ftdi_new()) == 0)
    {
        fprintf(stderr, "ftdi_new failed\n");
        return EXIT_FAILURE;
    }

    if ((ret = ftdi_usb_find_all(ftdi, &devlist, 0, 0)) < 0)
    {
        fprintf(stderr, "ftdi_usb_find_all failed: %d (%s)\n", ret, ftdi_get_error_string(ftdi));
        retval =  EXIT_FAILURE;
        goto do_deinit;
    }

    printf("Number of FTDI devices found: %d\n", ret);

    i = 0;
    for (curdev = devlist; curdev != NULL; i++)
    {
        printf("Checking device: %d\n", i);
        if ((ret = ftdi_usb_get_strings(ftdi, curdev->dev, manufacturer, 128, description, 128, NULL, 0)) < 0)
        {
            fprintf(stderr, "ftdi_usb_get_strings failed: %d (%s)\n", ret, ftdi_get_error_string(ftdi));
            retval = EXIT_FAILURE;
            goto done;
        }
        printf("Manufacturer: %s, Description: %s\n\n", manufacturer, description);
        curdev = curdev->next;
    }
done:
    ftdi_list_free(&devlist);
do_deinit:
    ftdi_free(ftdi);

    return retval;
}
