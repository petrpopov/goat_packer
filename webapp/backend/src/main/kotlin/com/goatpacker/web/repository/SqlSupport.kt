package com.goatpacker.web.repository

/** Значения перечисления для `in (...)` в SQL: `('QUEUED','RUNNING')`. Имена enum — не пользовательский ввод. */
fun Collection<Enum<*>>.sqlList(): String = joinToString(",", "(", ")") { "'${it.name}'" }
