"""Pytest configuration and shared fixtures."""

import pytest
from typing import Generator
from mypackage.models import Config, User, Admin
from mypackage.repositories import InMemoryUserRepository


@pytest.fixture(scope="session")
def test_config() -> Config:
    """Session-scoped test configuration."""
    return Config(
        database_url="sqlite:///:memory:",
        api_key="test-api-key",
        debug=True,
        max_connections=10,
    )


@pytest.fixture
def user_repository() -> Generator[InMemoryUserRepository, None, None]:
    """Provide clean user repository for each test."""
    repo = InMemoryUserRepository()
    yield repo
    repo.clear()


@pytest.fixture
def sample_users(user_repository: InMemoryUserRepository) -> list[User]:
    """Create sample users in repository."""
    users = [
        User(1, "alice", "alice@example.com"),
        User(2, "bob", "bob@example.com"),
        User(3, "charlie", "charlie@example.com"),
    ]
    for user in users:
        user_repository.save(user)
    return users


@pytest.fixture
def admin_user() -> Admin:
    """Provide admin user for tests requiring elevated privileges."""
    return Admin(
        entity_id=100,
        username="superadmin",
        email="superadmin@example.com",
        permissions=["read", "write", "delete", "manage_users"],
    )


@pytest.fixture(autouse=True)
def reset_entity_registry() -> Generator[None, None, None]:
    """Reset entity registry after each test."""
    from mypackage.models import Entity
    yield
    Entity._registry.clear()
