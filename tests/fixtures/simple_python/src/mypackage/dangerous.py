"""Module with dangerous operations for safety testing.

WARNING: This module intentionally contains dangerous patterns
that charter's safety analysis should detect.
"""

import subprocess
import pickle
import os
from typing import Any, Dict, Optional


def execute_dynamic_code(code: str) -> Any:
    """Execute arbitrary Python code. DANGEROUS!"""
    return exec(code)


def evaluate_expression(expression: str, context: Optional[Dict[str, Any]] = None) -> Any:
    """Evaluate arbitrary Python expression. DANGEROUS!"""
    return eval(expression, context or {})


def run_shell_command(command: str) -> str:
    """Run arbitrary shell command. DANGEROUS!"""
    result = subprocess.run(
        command,
        shell=True,
        capture_output=True,
        text=True,
    )
    return result.stdout


def run_command_list(args: list[str]) -> subprocess.CompletedProcess[str]:
    """Run command with argument list."""
    return subprocess.run(args, capture_output=True, text=True)


def call_external_program(program: str, *args: str) -> int:
    """Call external program using subprocess.call. DANGEROUS!"""
    return subprocess.call([program, *args])


def spawn_process(command: str) -> subprocess.Popen[bytes]:
    """Spawn a subprocess. DANGEROUS!"""
    return subprocess.Popen(command, shell=True, stdout=subprocess.PIPE)


def serialize_object(obj: Any, filepath: str) -> None:
    """Serialize object using pickle. DANGEROUS for untrusted data!"""
    with open(filepath, "wb") as file:
        pickle.dump(obj, file)


def deserialize_object(filepath: str) -> Any:
    """Deserialize object using pickle. DANGEROUS for untrusted data!"""
    with open(filepath, "rb") as file:
        return pickle.load(file)


def pickle_roundtrip(obj: Any) -> Any:
    """Pickle and unpickle object."""
    data = pickle.dumps(obj)
    return pickle.loads(data)


class DynamicLoader:
    """Class that dynamically loads and executes code."""

    def __init__(self, code_registry: Dict[str, str]) -> None:
        self.code_registry = code_registry
        self._compiled_cache: Dict[str, Any] = {}

    def register_code(self, name: str, code: str) -> None:
        """Register code snippet for later execution."""
        self.code_registry[name] = code

    def execute_registered(self, name: str, globals_dict: Optional[Dict[str, Any]] = None) -> Any:
        """Execute registered code by name. DANGEROUS!"""
        if name not in self.code_registry:
            raise KeyError(f"No code registered for '{name}'")

        code = self.code_registry[name]
        exec(code, globals_dict or {})

    def compile_and_cache(self, name: str) -> Any:
        """Compile code and cache the result."""
        if name not in self._compiled_cache:
            code = self.code_registry.get(name, "")
            self._compiled_cache[name] = compile(code, f"<{name}>", "exec")
        return self._compiled_cache[name]

    def eval_expression(self, expression: str) -> Any:
        """Evaluate expression with registered code context. DANGEROUS!"""
        context: Dict[str, Any] = {}
        for name, code in self.code_registry.items():
            exec(code, context)
        return eval(expression, context)


class PluginExecutor:
    """Execute plugin code from untrusted sources."""

    def __init__(self) -> None:
        self.plugins: Dict[str, str] = {}

    def load_plugin(self, name: str, code: str) -> None:
        """Load plugin code."""
        self.plugins[name] = code

    def run_plugin(self, name: str, **kwargs: Any) -> Any:
        """Run plugin with given arguments. DANGEROUS!"""
        if name not in self.plugins:
            raise ValueError(f"Plugin '{name}' not found")

        local_vars: Dict[str, Any] = {"kwargs": kwargs, "result": None}
        exec(self.plugins[name], {}, local_vars)
        return local_vars.get("result")


def system_command(command: str) -> int:
    """Execute system command using os.system. DANGEROUS!"""
    return os.system(command)


def read_with_eval(filepath: str) -> Any:
    """Read file and evaluate contents. EXTREMELY DANGEROUS!"""
    with open(filepath, "r") as file:
        content = file.read()
    return eval(content)


def create_function_from_string(func_code: str) -> Any:
    """Create function from string. DANGEROUS!"""
    local_vars: Dict[str, Any] = {}
    exec(func_code, {}, local_vars)
    return local_vars.get("func")
