/* final_all_pp.cpp

   Simple libftdi-cpp usage

   This program is distributed under the GPL, version 2
*/

#include "ftdi.hpp"
#include <iostream>
#include <iomanip>
#include <cstdlib>
#include <cstring>
using namespace Ftdi;

int main(int argc, char **argv)
{
    // Show help
    if (argc > 1)
    {
        if (strcmp(argv[1],"-h") == 0 || strcmp(argv[1],"--help") == 0)
        {
            std::cout << "Usage: " << argv[0] << " [-v VENDOR_ID] [-p PRODUCT_ID]" << std::endl;
            return EXIT_SUCCESS;
        }
    }

    // Parse args
    int vid = 0x0403, pid = 0x6010, tmp = 0;
    for (int i = 0; i < (argc - 1); i++)
    {
        if (strcmp(argv[i], "-v") == 0)
            if ((tmp = strtol(argv[++i], 0, 16)) >= 0)
                vid = tmp;

        if (strcmp(argv[i], "-p") == 0)
            if ((tmp = strtol(argv[++i], 0, 16)) >= 0)
                pid = tmp;
    }

    // Print header
    std::cout << std::hex << std::showbase
    << "Found devices ( VID: " << vid << ", PID: " << pid << " )"
    << std::endl
    << "------------------------------------------------"
    << std::endl << std::dec;

    // Print whole list
    Context context;
    List* list = List::find_all(context, vid, pid);
    for (List::iterator it = list->begin(); it != list->end(); it++)
    {
        std::cout << "FTDI (" << &*it << "): "
        << it->vendor() << ", "
        << it->description() << ", "
        << it->serial();

        // Open test
        if(it->open() == 0)
           std::cout << " (Open OK)";
        else
           std::cout << " (Open FAILED)";

        it->close();

        std::cout << std::endl;

    }

    delete list;

    return EXIT_SUCCESS;
}
