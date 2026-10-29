"""Cython 与运行时依赖之间的兼容垫片。

Cython 把普通函数/方法编译成 ``cython_function_or_method``，它既不是
``types.FunctionType``，也不在 Pydantic 2.x 的 ``default_ignored_types()`` 里。
两处依赖这个身份判定，编译后都会失灵：

* Pydantic 建模时把 ``model_post_init`` / ``to_public_dict`` 这类方法当成
  「未标注的字段」并拒绝建类 —— 实测 2026-10-28 的整树编译里有 546 个模块
  因此无法导入、被构建脚本退回明文，等于编译白做。
* Starlette 的 ``Route`` 用 ``inspect.isfunction()`` 区分「函数端点」与
  「ASGI 类」，判定失败就把端点当 ASGI 直接以 ``(scope, receive, send)``
  调用 —— 见 ``install_starlette_route_compat``。

两个安装函数在纯 Python 运行时都是空操作。
"""

import inspect as _std_inspect
from types import FunctionType

_installed = False
_routing_installed = False


def install_cython_function_compat() -> bool:
    """让 Pydantic 忽略 Cython 编译后的方法；返回是否真的打了补丁。"""
    global _installed
    if _installed:
        return True
    if type(install_cython_function_compat) is FunctionType:
        return False

    from pydantic._internal import _model_construction

    cython_function = type(install_cython_function_compat)
    original = _model_construction.default_ignored_types

    def default_ignored_types():
        types = original()
        return types if cython_function in types else types + (cython_function,)

    _model_construction.default_ignored_types = default_ignored_types
    _installed = True
    return True


_CYTHON_FUNCTION_TYPE_NAME = "cython_function_or_method"


def _is_cython_function(obj: object) -> bool:
    """按类型名判定，而不是 ``isinstance``。

    ``isinstance`` 要先拿到 cyfunction 类型本身，而那个类型只能从「已经被
    编译的某个东西」上取；本垫片是**无条件安装**的（纯 Python 运行时必须
    无害），安装时进程里可能一个编译模块都还没有。类型名在 Cython 3.x 内
    稳定；万一将来改名，症状会退回到「端点收到三个位置参数」的 500，由
    发布层的侧车冒烟（packaged sidecar 的 ``/api/config`` 必须 200）兜住。
    """
    return callable(obj) and type(obj).__name__ == _CYTHON_FUNCTION_TYPE_NAME


class _RoutingInspect:
    """``starlette.routing.inspect`` 的代理：只改写两个身份判定，其余原样转发。"""

    _native_compat_shim = True

    def __getattr__(self, name: str):
        return getattr(_std_inspect, name)

    @staticmethod
    def isfunction(obj: object) -> bool:
        return _std_inspect.isfunction(obj) or _is_cython_function(obj)

    @staticmethod
    def ismethod(obj: object) -> bool:
        return _std_inspect.ismethod(obj) or _is_cython_function(obj)


def install_starlette_route_compat() -> bool:
    """让 ``starlette.routing.Route`` 认识编译后的函数端点。

    Starlette 的真身逻辑是：函数端点包一层 ``request_response()``（收到
    ``Request``），其它一律当 ASGI 直接调用。Cython 编译后 ``health`` 这类
    闭包既非 ``isfunction`` 也非 ``ismethod``，于是每个 ``Route`` 都掉进
    ASGI 分支。实测 2026-10-29 的编译树：网关能起来、``/health`` 却返回
    ``health() takes exactly 1 positional argument (3 given)``。

    ``is_async_callable`` 不需要垫 —— Cython 保留了协程标记，编译后的
    ``async def`` 仍被 ``asyncio.iscoroutinefunction`` 认作协程函数。所以
    补上身份判定后，其余完全走 Starlette 原路径。
    """
    global _routing_installed
    if _routing_installed:
        return True
    try:
        from starlette import routing
    except ImportError:  # 只有 CLI 的极简环境可能没有 starlette
        return False
    if getattr(routing.inspect, "_native_compat_shim", False):
        _routing_installed = True
        return True

    routing.inspect = _RoutingInspect()
    _routing_installed = True
    return True
