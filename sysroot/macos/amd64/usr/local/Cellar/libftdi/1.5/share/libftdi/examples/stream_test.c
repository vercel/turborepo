/* stream_test.c
 *
 * Test reading  from FT2232H in synchronous FIFO mode.
 *
 * The FT2232H must supply data due to an appropriate circuit
 *
 * To check for skipped block with appended code, 
 *     a structure as follows is assumed
 * 1* uint32_t num (incremented in 0x4000 steps)
 * 3* uint32_t dont_care
 *
 * After start, data will be read in streaming until the program is aborted
 * Progress information will be printed out
 * If a filename is given on the command line, the data read will be
 * written to that file
 *
 */
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>
#include <getopt.h>
#include <signal.h>
#include <errno.h>
#include <ftdi.h>
void check_outfile(char *);

static FILE *outputFile;

static int check = 1;
static int exitRequested = 0;
/*
 * sigintHandler --
 *
 *    SIGINT handler, so we can gracefully exit when the user hits ctrl-C.
 */

static void
sigintHandler(int signum)
{
   exitRequested = 1;
}

static void
usage(const char *argv0)
{
   fprintf(stderr,
           "Usage: %s [options...] \n"
           "Test streaming read from FT2232H\n"
           "[-P string] only look for product with given string\n"
           "[-n] don't check for special block structure\n"
           "\n"
           "If some filename is given, write data read to that file\n"
           "Progress information is printed each second\n"
           "Abort with ^C\n"
           "\n"
           "Options:\n"
           "\n"
           "Copyright (C) 2009 Micah Dowty <micah@navi.cx>\n"
           "Adapted for use with libftdi (C) 2010 Uwe Bonnes <bon@elektron.ikp.physik.tu-darmstadt.de>\n",
           argv0);
   exit(1);
}

static uint32_t start = 0;
static uint32_t offset = 0;
static uint64_t blocks = 0;
static uint32_t skips = 0;
static uint32_t n_err = 0;
static int
readCallback(uint8_t *buffer, int length, FTDIProgressInfo *progress, void *userdata)
{
   if (length)
   {
       if (check)
       {
           int i,rem;
           uint32_t num;
           for (i= offset; i<length-16; i+=16)
           {
               num = *(uint32_t*) (buffer+i);
               if (start && (num != start +0x4000))
               {
                   uint32_t delta = ((num-start)/0x4000)-1;
                   fprintf(stderr, "Skip %7d blocks from 0x%08x to 0x%08x at blocks %10llu\n",
                           delta, start -0x4000, num, (unsigned long long)blocks);
                   n_err++;
                   skips += delta;
               }
               blocks ++;
               start = num;
           }
           rem = length -i;
           if (rem >3)
           {
               num = *(uint32_t*) (buffer+i);
               if (start && (num != start +0x4000))
               {
                   uint32_t delta = ((num-start)/0x4000)-1;
                   fprintf(stderr, "Skip %7d blocks from 0x%08x to 0x%08x at blocks %10llu\n",
                           delta, start -0x4000, num, (unsigned long long) blocks);
                   n_err++;
                   skips += delta;
               }
               start = num;
           }
           else if (rem)
               start += 0x4000;
           if (rem != 0)
           {
               blocks ++;
               offset = 16-rem;
           }
       }
       if (outputFile)
       {
           if (fwrite(buffer, length, 1, outputFile) != 1)
           {
               perror("Write error");
               return 1;
           }
       }
   }
   if (progress)
   {
       fprintf(stderr, "%10.02fs total time %9.3f MiB captured %7.1f kB/s curr rate %7.1f kB/s totalrate %d dropouts\n",
               progress->totalTime,
               progress->current.totalBytes / (1024.0 * 1024.0),
               progress->currentRate / 1024.0,
               progress->totalRate / 1024.0,
               n_err);
   }
   return exitRequested ? 1 : 0;
}

int main(int argc, char **argv)
{
   struct ftdi_context *ftdi;
   int err, c;
   FILE *of = NULL;
   char const *outfile  = 0;
   outputFile =0;
   exitRequested = 0;
   char *descstring = NULL;
   int option_index;
   static struct option long_options[] = {{NULL},};

   while ((c = getopt_long(argc, argv, "P:n", long_options, &option_index)) !=- 1)
       switch (c) 
       {
       case -1:
           break;
       case 'P':
           descstring = optarg;
           break;
       case 'n':
           check = 0;
           break;
       default:
           usage(argv[0]);
       }
   
   if (optind == argc - 1)
   {
       // Exactly one extra argument- a dump file
       outfile = argv[optind];
   }
   else if (optind < argc)
   {
       // Too many extra args
       usage(argv[0]);
   }
   
   if ((ftdi = ftdi_new()) == 0)
   {
       fprintf(stderr, "ftdi_new failed\n");
       return EXIT_FAILURE;
   }
   
   if (ftdi_set_interface(ftdi, INTERFACE_A) < 0)
   {
       fprintf(stderr, "ftdi_set_interface failed\n");
       ftdi_free(ftdi);
       return EXIT_FAILURE;
   }
   
   if (ftdi_usb_open_desc(ftdi, 0x0403, 0x6010, descstring, NULL) < 0)
   {
       fprintf(stderr,"Can't open ftdi device: %s\n",ftdi_get_error_string(ftdi));
       ftdi_free(ftdi);
       return EXIT_FAILURE;
   }
   
   /* A timeout value of 1 results in may skipped blocks */
   if(ftdi_set_latency_timer(ftdi, 2))
   {
       fprintf(stderr,"Can't set latency, Error %s\n",ftdi_get_error_string(ftdi));
       ftdi_usb_close(ftdi);
       ftdi_free(ftdi);
       return EXIT_FAILURE;
   }
   
/*   if(ftdi_usb_purge_rx_buffer(ftdi) < 0)
   {
       fprintf(stderr,"Can't rx purge\n",ftdi_get_error_string(ftdi));
       return EXIT_FAILURE;
       }*/
   if (outfile)
       if ((of = fopen(outfile,"w+")) == 0)
           fprintf(stderr,"Can't open logfile %s, Error %s\n", outfile, strerror(errno));
   if (of)
       if (setvbuf(of, NULL, _IOFBF , 1<<16) == 0)
           outputFile = of;
   signal(SIGINT, sigintHandler);
   
   err = ftdi_readstream(ftdi, readCallback, NULL, 8, 256);
   if (err < 0 && !exitRequested)
       exit(1);
   
   if (outputFile) {
       fclose(outputFile);
       outputFile = NULL;
   }
   fprintf(stderr, "Capture ended.\n");
   
   if (ftdi_set_bitmode(ftdi,  0xff, BITMODE_RESET) < 0)
   {
       fprintf(stderr,"Can't set synchronous fifo mode, Error %s\n",ftdi_get_error_string(ftdi));
       ftdi_usb_close(ftdi);
       ftdi_free(ftdi);
       return EXIT_FAILURE;
   }
   ftdi_usb_close(ftdi);
   ftdi_free(ftdi);
   signal(SIGINT, SIG_DFL);
   if (check && outfile)
   {
       if ((outputFile = fopen(outfile,"r")) == 0)
       {
           fprintf(stderr,"Can't open logfile %s, Error %s\n", outfile, strerror(errno));
           ftdi_usb_close(ftdi);
           ftdi_free(ftdi);
           return EXIT_FAILURE;
       }
       check_outfile(descstring);
       fclose(outputFile);
   }
   else if (check)
       fprintf(stderr,"%d errors of %llu blocks (%Le), %d (%Le) blocks skipped\n",
               n_err, (unsigned long long) blocks, (long double)n_err/(long double) blocks,
               skips, (long double)skips/(long double) blocks);
   exit (0);
}

void check_outfile(char *descstring)
{
    if(strcmp(descstring,"FT2232HTEST") == 0)
    {
       char buf0[1024];
       char buf1[1024];
       char bufr[1024];
       char *pa, *pb, *pc;
       unsigned int num_lines = 0, line_num = 1;
       int err_count = 0;
       unsigned int num_start, num_end;

       pa = buf0;
       pb = buf1;
       pc = buf0;
       if(fgets(pa, 1023, outputFile) == NULL)
       {
           fprintf(stderr,"Empty output file\n");
           return;
       }
       while(fgets(pb, 1023, outputFile) != NULL)
       {
           num_lines++;
           unsigned int num_save = num_start;
           if( sscanf(pa,"%6u%94s%6u",&num_start, bufr,&num_end) !=3)
           {
               fprintf(stdout,"Format doesn't match at line %8d \"%s",
                       num_lines, pa);
               err_count++;
               line_num = num_save +2;
           }
           else
           {
               if ((num_start+1)%100000 != num_end)
               {
                   if (err_count < 20)
                       fprintf(stdout,"Malformed line %d \"%s\"\n", 
                               num_lines, pa);
                   err_count++;
               }
               else if(num_start != line_num)
               {
                   if (err_count < 20)
                       fprintf(stdout,"Skipping from %d to %d\n", 
                               line_num, num_start);
                   err_count++;
                  
               }
               line_num = num_end;
           }
           pa = pb;
           pb = pc;
           pc = pa;
       }
       if(err_count)
           fprintf(stdout,"\n%d errors of %d data sets %f\n", err_count, num_lines, (double) err_count/(double)num_lines);
       else
           fprintf(stdout,"No errors for %d lines\n",num_lines);
   }
    else if(strcmp(descstring,"LLBBC10") == 0)
    { 
        uint32_t block0[4];
        uint32_t block1[4];
        uint32_t *pa = block0;
        uint32_t *pb = block1;
        uint32_t *pc = block0;
        uint32_t start= 0;
        uint32_t nread = 0;
        int n_shown = 0;
        int n_errors = 0;
        if (fread(pa, sizeof(uint32_t), 4,outputFile) < 4)
        {
            fprintf(stderr,"Empty result file\n");
            return;
        }
        while(fread(pb, sizeof(uint32_t), 4,outputFile) != 0)
        {
            blocks++;
            nread =  pa[0];
            if(start>0 && (nread != start))
            {
                if(n_shown < 30)
                {
                    fprintf(stderr, "Skip %7d blocks from 0x%08x to 0x%08x at blocks %10llu \n",
                            (nread-start)/0x4000, start -0x4000, nread, (unsigned long long) blocks);
                    n_shown ++;
                }
                n_errors++;
            }
            else if (n_shown >0) 
                n_shown--; 
            start = nread + 0x4000;
            pa = pb;
            pb = pc;
            pc = pa;
        }
        if(n_errors)
            fprintf(stderr, "%d blocks wrong from %llu blocks read\n",
                    n_errors, (unsigned long long) blocks);
        else
            fprintf(stderr, "%llu blocks all fine\n", (unsigned long long) blocks);
    }
}
