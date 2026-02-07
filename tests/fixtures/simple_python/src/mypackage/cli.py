"""Command-line interface for mypackage."""

import argparse
import sys
from typing import Optional, List


def parse_args(args: Optional[List[str]] = None) -> argparse.Namespace:
    """Parse command-line arguments."""
    parser = argparse.ArgumentParser(
        prog="mypackage",
        description="MyPackage CLI tool",
    )
    parser.add_argument(
        "-v", "--verbose",
        action="store_true",
        help="Enable verbose output",
    )
    parser.add_argument(
        "--config",
        type=str,
        help="Path to configuration file",
    )

    subparsers = parser.add_subparsers(dest="command", help="Available commands")

    user_parser = subparsers.add_parser("user", help="User management")
    user_parser.add_argument("action", choices=["list", "create", "delete"])
    user_parser.add_argument("--username", type=str)
    user_parser.add_argument("--email", type=str)

    return parser.parse_args(args)


def main(args: Optional[List[str]] = None) -> int:
    """Main entry point for CLI."""
    parsed = parse_args(args)

    if parsed.verbose:
        print("Verbose mode enabled")

    if parsed.command == "user":
        return handle_user_command(parsed)

    print("No command specified. Use --help for usage.")
    return 0


def handle_user_command(args: argparse.Namespace) -> int:
    """Handle user subcommand."""
    if args.action == "list":
        print("Listing users...")
        return 0
    elif args.action == "create":
        if not args.username or not args.email:
            print("Error: --username and --email required for create")
            return 1
        print(f"Creating user: {args.username}")
        return 0
    elif args.action == "delete":
        if not args.username:
            print("Error: --username required for delete")
            return 1
        print(f"Deleting user: {args.username}")
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
