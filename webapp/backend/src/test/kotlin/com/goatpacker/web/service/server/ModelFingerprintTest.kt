package com.goatpacker.web.service.server

import com.goatpacker.web.descriptorProject3mf
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNotEquals
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.nio.file.Files
import java.nio.file.Path

class ModelFingerprintTest {
    @TempDir lateinit var dir: Path

    private fun mesh(z: String = "5", attributes: String = """paint_supports="4" paint_color="8"""", uuid: String = "0001") =
        """<object id="1" p:UUID="$uuid-81cb" type="model"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/>""" +
            """<vertex x="0" y="1" z="$z"/></vertices><triangles><triangle v1="0" v2="1" v3="2" $attributes/></triangles></mesh></object>"""

    private fun model(resources: String, metadata: String = "a", build: String = "<build/>") =
        """<model xmlns:p="urn:p"><metadata name="Date">$metadata</metadata><resources>$resources</resources>$build</model>"""

    private fun file(name: String, bytes: ByteArray): Path = dir.resolve(name).also { Files.write(it, bytes) }

    private fun project(name: String, inline: String? = null, objects: Map<String, String> = emptyMap(), extra: Map<String, String> = emptyMap()) =
        file(name, descriptorProject3mf("P", model = inline ?: model(""), extraEntries = objects + extra))

    private fun fingerprint(path: Path) = ModelFingerprint.of(path, isStl = false)

    @Test fun `metadata, thumbnails, build, uuid, attribute order and number spelling do not change the group`() {
        val a = project("a.3mf", inline = model(mesh(), "2026-10-31", """<build><item objectid="1"/></build>"""))
        val b = project(
            "b.3mf",
            inline = model(mesh(z = "5.000", attributes = """paint_color="8" paint_supports="4"""", uuid = "0004"), "2026-11-03"),
            extra = mapOf("Metadata/plate_1.png" to "other", "Metadata/top_1.png" to "new"),
        )
        assertEquals(fingerprint(a), fingerprint(b))
    }

    @Test fun `the same mesh in a separate object part and inline gives one group, empty parts are ignored`() {
        val inline = project("i.3mf", inline = model(mesh()))
        val parts = project(
            "p.3mf",
            objects = mapOf("3D/Objects/object_7.model" to model(mesh()), "3D/Objects/object_8.model" to model("")),
        )
        assertEquals(fingerprint(inline), fingerprint(parts))
    }

    @Test fun `a changed vertex, paint or an extra object changes the group`() {
        val base = fingerprint(project("a.3mf", inline = model(mesh())))
        assertNotEquals(base, fingerprint(project("v.3mf", inline = model(mesh(z = "5.5")))))
        assertNotEquals(base, fingerprint(project("c.3mf", inline = model(mesh(attributes = """paint_color="4"""")))))
        assertNotEquals(base, fingerprint(project("o.3mf", inline = model(mesh()), objects = mapOf("3D/Objects/o.model" to model(mesh(z = "9"))))))
    }

    @Test fun `stl is the hash of its bytes`() {
        val a = file("a.stl", "solid a".toByteArray())
        assertEquals(ModelFingerprint.of(a, true), ModelFingerprint.of(file("b.stl", "solid a".toByteArray()), true))
        assertNotEquals(ModelFingerprint.of(a, true), ModelFingerprint.of(file("c.stl", "solid b".toByteArray()), true))
    }

    @Test fun `without a mesh or without a readable archive the whole file is hashed`() {
        val bare = project("bare.3mf")
        val changed = project("bare2.3mf", extra = mapOf("Metadata/plate_1.png" to "other"))
        assertNotEquals(fingerprint(bare), fingerprint(changed))
        val broken = file("broken.3mf", "not a zip".toByteArray())
        assertEquals(fingerprint(broken), fingerprint(file("broken2.3mf", "not a zip".toByteArray())))
        assertNotEquals(fingerprint(broken), fingerprint(file("broken3.3mf", "not a zip!".toByteArray())))
    }
}
