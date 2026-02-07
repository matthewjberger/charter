"""Custom exception classes for the application."""

from typing import Optional, Dict, Any, List


class BaseError(Exception):
    """Base exception for all application errors."""

    def __init__(
        self,
        message: str,
        code: Optional[str] = None,
        details: Optional[Dict[str, Any]] = None,
    ) -> None:
        super().__init__(message)
        self.message = message
        self.code = code or self.__class__.__name__
        self.details = details or {}

    def to_dict(self) -> Dict[str, Any]:
        """Convert exception to dictionary for API responses."""
        return {
            "error": self.code,
            "message": self.message,
            "details": self.details,
        }


class ValidationError(BaseError):
    """Raised when input validation fails."""

    def __init__(
        self,
        message: str,
        field: Optional[str] = None,
        errors: Optional[List[str]] = None,
    ) -> None:
        super().__init__(message, "VALIDATION_ERROR", {"field": field, "errors": errors or []})
        self.field = field
        self.errors = errors or []


class NotFoundError(BaseError):
    """Raised when a requested resource is not found."""

    def __init__(
        self,
        resource_type: str,
        resource_id: Any,
    ) -> None:
        message = f"{resource_type} with id '{resource_id}' not found"
        super().__init__(message, "NOT_FOUND", {"resource_type": resource_type, "resource_id": resource_id})
        self.resource_type = resource_type
        self.resource_id = resource_id


class AuthenticationError(BaseError):
    """Raised when authentication fails."""

    def __init__(self, message: str = "Authentication failed") -> None:
        super().__init__(message, "AUTHENTICATION_ERROR")


class AuthorizationError(BaseError):
    """Raised when user lacks required permissions."""

    def __init__(
        self,
        required_permission: str,
        user_permissions: Optional[List[str]] = None,
    ) -> None:
        message = f"Missing required permission: {required_permission}"
        super().__init__(
            message,
            "AUTHORIZATION_ERROR",
            {"required": required_permission, "user_permissions": user_permissions or []},
        )
        self.required_permission = required_permission
        self.user_permissions = user_permissions or []


class RateLimitError(BaseError):
    """Raised when rate limit is exceeded."""

    def __init__(
        self,
        limit: int,
        window_seconds: int,
        retry_after: Optional[int] = None,
    ) -> None:
        message = f"Rate limit of {limit} requests per {window_seconds}s exceeded"
        super().__init__(
            message,
            "RATE_LIMIT_ERROR",
            {"limit": limit, "window": window_seconds, "retry_after": retry_after},
        )
        self.limit = limit
        self.window_seconds = window_seconds
        self.retry_after = retry_after


class DatabaseError(BaseError):
    """Raised when database operations fail."""

    def __init__(
        self,
        operation: str,
        message: str,
        original_error: Optional[Exception] = None,
    ) -> None:
        super().__init__(
            f"Database {operation} failed: {message}",
            "DATABASE_ERROR",
            {"operation": operation},
        )
        self.operation = operation
        self.original_error = original_error


class ConfigurationError(BaseError):
    """Raised when configuration is invalid or missing."""

    def __init__(
        self,
        config_key: str,
        message: str,
    ) -> None:
        super().__init__(
            f"Configuration error for '{config_key}': {message}",
            "CONFIGURATION_ERROR",
            {"config_key": config_key},
        )
        self.config_key = config_key


def raise_if_none(value: Any, error_message: str) -> Any:
    """Raise ValidationError if value is None."""
    if value is None:
        raise ValidationError(error_message)
    return value


def raise_not_found(resource_type: str, resource_id: Any) -> None:
    """Convenience function to raise NotFoundError."""
    raise NotFoundError(resource_type, resource_id)


def assert_permission(
    user_permissions: List[str],
    required: str,
) -> None:
    """Assert user has required permission or raise AuthorizationError."""
    if required not in user_permissions:
        raise AuthorizationError(required, user_permissions)
