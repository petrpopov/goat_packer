package com.goatpacker.web.util

/**
 * Подготовительный шаг + основная команда одним процессом: `/bin/sh -c 'prelude && exec main'`.
 * Один процесс — потому что в docker-режиме у джоба один контейнер (tmpfs /work живёт до его
 * остановки), а отмена, таймаут и разбор вывода уже завязаны на один процесс. `exec` оставляет
 * основной команде pid оболочки: сигналы и код выхода — её. Без подготовки команда идёт как есть.
 */
object ShellChain {
    fun quote(arg: String): String = "'" + arg.replace("'", "'\\''") + "'"

    fun script(prelude: List<String>, main: List<String>): String =
        prelude.joinToString(" ", transform = ::quote) + " && exec " + main.joinToString(" ", transform = ::quote)

    fun command(prelude: List<String>?, main: List<String>): List<String> =
        if (prelude == null) main else listOf("/bin/sh", "-c", script(prelude, main))
}
