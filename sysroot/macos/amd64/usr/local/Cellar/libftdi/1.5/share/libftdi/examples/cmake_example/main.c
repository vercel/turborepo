/* main.c

   Example for ftdi_new()

   This program is distributed under the GPL, version 2
*/

#include <stdio.h>
#include <stdlib.h>
#include <ftdi.h>

int main(void)
{
  struct ftdi_context *ftdi;
  int retval = EXIT_SUCCESS;

  if ((ftdi = ftdi_new()) == 0)
  {
    fprintf(stderr, "ftdi_new failed\n");
    return EXIT_FAILURE;
  }
  
  return retval;
}
