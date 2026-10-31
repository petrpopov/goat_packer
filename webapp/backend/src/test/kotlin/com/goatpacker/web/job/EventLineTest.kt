package com.goatpacker.web.job

import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test

class EventLineTest {
    @Test fun `json events are recognised and human lines are not`() {
        assertTrue(JobRunner.isEventLine("""{"event":"stage","id":"pack"}"""))
        assertTrue(JobRunner.isEventLine("""  {"event":"slicer_progress","elapsed_s":4}  """))
        assertFalse(JobRunner.isEventLine("Error: slicer failed"))
        assertFalse(JobRunner.isEventLine("""{"not":"an event"}"""))
        assertFalse(JobRunner.isEventLine("""message with "event" inside"""))
        assertTrue(JobRunner.isLayoutEvent("""{"event":"layout","placements":[]}"""))
        assertFalse(JobRunner.isLayoutEvent("""{"event":"stage"}"""))
    }
}
