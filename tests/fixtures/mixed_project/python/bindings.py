"""Python bindings for the Rust library."""

from typing import Protocol


class Calculator(Protocol):
    """Protocol matching Rust Calculator trait."""

    def calculate(self, input: int) -> int:
        """Calculate result from input."""
        ...


class PythonWrapper:
    """Python wrapper for RustType."""

    def __init__(self, value: int) -> None:
        self.value = value

    def double(self) -> int:
        """Double the value."""
        return self.value * 2

    def calculate(self, input: int) -> int:
        """Calculate value plus input."""
        return self.value + input
