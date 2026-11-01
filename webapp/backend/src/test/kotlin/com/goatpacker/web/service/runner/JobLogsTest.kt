package com.goatpacker.web.service.runner

import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test

class JobLogsTest {
    @Test fun `json events are recognised and human lines are not`() {
        assertTrue(JobLogs.isEventLine("""{"event":"stage","id":"pack"}"""))
        assertTrue(JobLogs.isEventLine("""  {"event":"slicer_progress","elapsed_s":4}  """))
        assertFalse(JobLogs.isEventLine("Error: slicer failed"))
        assertFalse(JobLogs.isEventLine("""{"not":"an event"}"""))
        assertFalse(JobLogs.isEventLine("""message with "event" inside"""))
        assertTrue(JobLogs.isLayoutEvent("""{"event":"layout","placements":[]}"""))
        assertFalse(JobLogs.isLayoutEvent("""{"event":"stage"}"""))
    }
}
