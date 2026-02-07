"""
MyPackage - A comprehensive test package for charter Python support.

This package demonstrates various Python constructs that charter should extract:
- Classes with inheritance and decorators
- Functions with type hints
- Async/await patterns
- Protocols and ABCs
- Error handling patterns
"""

from mypackage.models import User, Admin, Config
from mypackage.protocols import Serializable, Cacheable
from mypackage.services import UserService
from mypackage.errors import ValidationError, NotFoundError

__version__ = "1.0.0"
__all__ = [
    "User",
    "Admin",
    "Config",
    "Serializable",
    "Cacheable",
    "UserService",
    "ValidationError",
    "NotFoundError",
]
