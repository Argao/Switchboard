package com.aiyu.switchboard

import com.aiyu.switchboard.ui.system.Pending
import com.aiyu.switchboard.ui.system.SCROLL_STEP
import org.junit.Assert.assertEquals
import org.junit.Test

class TouchpadScrollTest {
    @Test fun barScrollsTheWayTheFingerSlides() {
        val p = Pending()
        p.barDrag(SCROLL_STEP * 3)
        assertEquals(3, p.takeScrollSteps())
        p.barDrag(-SCROLL_STEP * 2)
        assertEquals(-2, p.takeScrollSteps())
    }

    @Test fun twoFingersScrollAgainstTheDrag() {
        val p = Pending()
        p.twoFingerDrag(-SCROLL_STEP * 2)
        assertEquals(2, p.takeScrollSteps())
        p.twoFingerDrag(SCROLL_STEP)
        assertEquals(-1, p.takeScrollSteps())
    }

    @Test fun partOfAStepWaitsForTheRest() {
        val p = Pending()
        p.barDrag(SCROLL_STEP * 0.6f)
        assertEquals(0, p.takeScrollSteps())
        p.barDrag(SCROLL_STEP * 0.6f)
        assertEquals(1, p.takeScrollSteps())
        // Taken steps are gone; only the remaining fifth of a step is kept.
        assertEquals(0, p.takeScrollSteps())
        p.barDrag(SCROLL_STEP * 0.8f)
        assertEquals(1, p.takeScrollSteps())
    }

    @Test fun oppositeDragsCancelOut() {
        val p = Pending()
        p.barDrag(SCROLL_STEP * 2)
        p.barDrag(-SCROLL_STEP * 2)
        assertEquals(0, p.takeScrollSteps())
    }
}
