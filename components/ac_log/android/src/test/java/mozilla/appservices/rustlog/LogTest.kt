/* Any copyright is dedicated to the Public Domain.
   http://creativecommons.org/publicdomain/zero/1.0/ */

package mozilla.appservices.rustlog

import junit.framework.Assert
import org.junit.AfterClass
import org.junit.BeforeClass
import org.junit.rules.TemporaryFolder
import org.junit.Rule
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

import org.junit.Test
import org.junit.Assert.*
import java.lang.RuntimeException

@RunWith(RobolectricTestRunner::class)
@Config(manifest = Config.NONE)
class LogTest {

    fun writeTestLog(m: String) {
        LibRustLogAdapter.INSTANCE.ac_log_adapter_test__log_msg(m)
        Thread.sleep(100) // Wait for it to arrive...
    }

    @Test
    fun testLogging() {
        val logs: MutableList<String> = mutableListOf()

        assert(!RustLogAdapter.isEnabled)
        assert(RustLogAdapter.canEnable)

        RustLogAdapter.enable { level, tagStr, msgStr ->
            val threadId = Thread.currentThread().id
            val info = "Rust log from $threadId | Level: $level | tag: $tagStr | message: $msgStr"
            println(info)
            logs += info
        }

        // We log an informational message after initializing (but it's processed asynchronously).
        Thread.sleep(100)
        assertEquals(logs.size, 1)

        writeTestLog("Test123")

        assertEquals(logs.size, 2)

        assert(RustLogAdapter.isEnabled)
        assert(!RustLogAdapter.canEnable)
        var wasCalled = false;

        val didEnable = RustLogAdapter.tryEnable { _, _, _ ->
            wasCalled = true
        }

        assert(!didEnable);
        writeTestLog("Test456")

        assertEquals(logs.size, 3)
        assert(!wasCalled)
        RustLogAdapter.disable()
        assert(!RustLogAdapter.isEnabled)
        assert(!RustLogAdapter.canEnable)

        // Shouldn't do anything, we disabled the log.
        writeTestLog("Test789")

        assertEquals(logs.size, 3)
        assert(!wasCalled)


        val didEnable2 = RustLogAdapter.tryEnable { _, _, _ ->
            wasCalled = true
        }
        assert(!didEnable2)

        try {
            RustLogAdapter.enable { _, _, _ ->
                wasCalled = true
            }
            Assert.fail("enable should throw")
        } catch (e: RuntimeException) {
        }

        // One last time to make sure that those enable/tryEnable
        // calls didn't secretly work.
        writeTestLog("Test101112")
        assert(!wasCalled)
    }
}

