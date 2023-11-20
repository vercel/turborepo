#include <iostream>

int main(int argc, char *argv[])
{
    // Loop through the command-line arguments and print them with a space
    for (int i = 1; i < argc; i++)
    {
        std::cout << argv[i];
        // Print a space after each argument, except the last one
        if (i < argc - 1)
        {
            std::cout << " ";
        }
    }

    std::cout << std::endl;

    return 0;
}
