"""Validation utilities."""

import re
from typing import Optional, List
from mypackage.protocols import Validator


class EmailValidator(Validator[str]):
    """Validator for email addresses."""

    EMAIL_PATTERN = re.compile(
        r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$"
    )

    def __init__(self) -> None:
        self._errors: List[str] = []

    def validate(self, value: str) -> bool:
        """Validate email format."""
        self._errors = []

        if not value:
            self._errors.append("Email is required")
            return False

        if not self.EMAIL_PATTERN.match(value):
            self._errors.append("Invalid email format")
            return False

        if len(value) > 255:
            self._errors.append("Email exceeds maximum length")
            return False

        return True

    def get_errors(self) -> List[str]:
        """Get validation errors."""
        return self._errors


class UsernameValidator(Validator[str]):
    """Validator for usernames."""

    MIN_LENGTH = 3
    MAX_LENGTH = 50
    PATTERN = re.compile(r"^[a-zA-Z0-9_]+$")

    def __init__(
        self,
        min_length: Optional[int] = None,
        max_length: Optional[int] = None,
    ) -> None:
        self.min_length = min_length or self.MIN_LENGTH
        self.max_length = max_length or self.MAX_LENGTH
        self._errors: List[str] = []

    def validate(self, value: str) -> bool:
        """Validate username."""
        self._errors = []

        if not value:
            self._errors.append("Username is required")
            return False

        if len(value) < self.min_length:
            self._errors.append(f"Username must be at least {self.min_length} characters")

        if len(value) > self.max_length:
            self._errors.append(f"Username must be at most {self.max_length} characters")

        if not self.PATTERN.match(value):
            self._errors.append("Username can only contain letters, numbers, and underscores")

        return len(self._errors) == 0

    def get_errors(self) -> List[str]:
        """Get validation errors."""
        return self._errors


class PasswordValidator(Validator[str]):
    """Validator for passwords."""

    def __init__(
        self,
        min_length: int = 8,
        require_uppercase: bool = True,
        require_lowercase: bool = True,
        require_digit: bool = True,
        require_special: bool = False,
    ) -> None:
        self.min_length = min_length
        self.require_uppercase = require_uppercase
        self.require_lowercase = require_lowercase
        self.require_digit = require_digit
        self.require_special = require_special
        self._errors: List[str] = []

    def validate(self, value: str) -> bool:
        """Validate password strength."""
        self._errors = []

        if len(value) < self.min_length:
            self._errors.append(f"Password must be at least {self.min_length} characters")

        if self.require_uppercase and not any(c.isupper() for c in value):
            self._errors.append("Password must contain uppercase letter")

        if self.require_lowercase and not any(c.islower() for c in value):
            self._errors.append("Password must contain lowercase letter")

        if self.require_digit and not any(c.isdigit() for c in value):
            self._errors.append("Password must contain digit")

        if self.require_special:
            special_chars = "!@#$%^&*()_+-=[]{}|;':\",./<>?"
            if not any(c in special_chars for c in value):
                self._errors.append("Password must contain special character")

        return len(self._errors) == 0

    def get_errors(self) -> List[str]:
        """Get validation errors."""
        return self._errors


email_validator = EmailValidator()
username_validator = UsernameValidator()


def validate_all(validators: List[tuple[Validator[str], str]]) -> List[str]:
    """Run multiple validators and collect all errors."""
    all_errors: List[str] = []
    for validator, value in validators:
        validator.validate(value)
        all_errors.extend(validator.get_errors())
    return all_errors
