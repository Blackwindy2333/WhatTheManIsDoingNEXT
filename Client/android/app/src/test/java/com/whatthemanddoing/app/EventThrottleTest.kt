package com.whatthemanddoing.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class EventThrottleTest {

    @Test
    fun `same package and class within window is deduplicated`() {
        var now = 0L
        val throttle = EventThrottle(minIntervalMs = 300L) { now }
        assertTrue(throttle.shouldAccept("com.a", "Cls"))
        now = 299
        assertFalse(throttle.shouldAccept("com.a", "Cls"))
        assertTrue(throttle.acceptedCount == 1)
    }

    @Test
    fun `same package and class after window is accepted again`() {
        var now = 0L
        val throttle = EventThrottle(minIntervalMs = 300L) { now }
        assertTrue(throttle.shouldAccept("com.a", "Cls"))
        now = 300
        assertTrue(throttle.shouldAccept("com.a", "Cls"))
    }

    @Test
    fun `different package is always accepted immediately`() {
        var now = 0L
        val throttle = EventThrottle(minIntervalMs = 300L) { now }
        assertTrue(throttle.shouldAccept("com.a", "Cls"))
        now = 10
        assertTrue(throttle.shouldAccept("com.b", "Cls"))
        now = 20
        assertTrue(throttle.shouldAccept("com.a", "Other"))
    }

    @Test
    fun `empty class name is normalized like whitespace`() {
        var now = 0L
        val throttle = EventThrottle(minIntervalMs = 300L) { now }
        assertTrue(throttle.shouldAccept("com.a", ""))
        now = 50
        assertFalse(throttle.shouldAccept("com.a", " "))
    }
}
