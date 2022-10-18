// C:\Windows\Microsoft.NET\Framework\v4.0.30319\csc.exe -target:exe -platform:anycpu turbo.cs
namespace Turborepo
{
    class Executable {
        static int Main()
        {
            System.Console.Error.WriteLine("This OS/architecture combination is no longer supported for Turborepo. Please add a note to https://github.com/vercel/turborepo/discussions/1891 if this impacts your workflow.");
            return -1;
        }
    }
}
