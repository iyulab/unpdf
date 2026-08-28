using System.Runtime.CompilerServices;

// Test-only surface (raw-JSON accessors behind the strongly-typed DTOs) stays
// internal rather than public — this grants exactly the test project visibility,
// nothing else.
[assembly: InternalsVisibleTo("Unpdf.Tests")]
