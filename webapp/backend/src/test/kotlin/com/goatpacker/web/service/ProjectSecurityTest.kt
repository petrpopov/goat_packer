package com.goatpacker.web.service

import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper
import com.goatpacker.web.exception.BadRequestException
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Assertions.*
import java.nio.file.Files
import java.util.zip.*

class ProjectSecurityTest {
    private val inspector=Project3mf(jacksonObjectMapper())
    private fun project(entries:Map<String,String>,check:(java.nio.file.Path)->Unit) {
        val file=Files.createTempFile("self-contained-project-",".3mf")
        try {
            ZipOutputStream(Files.newOutputStream(file)).use { zip -> entries.forEach { (name,data)->zip.putNextEntry(ZipEntry(name));zip.write(data.toByteArray());zip.closeEntry() } }
            check(file)
        } finally {Files.deleteIfExists(file)}
    }
    @Test fun `reject traversal duplicate case XML entities and executable postprocessing`() {
        for(entries in listOf(
            mapOf("../outside" to "x"),
            mapOf("X.xml" to "<x/>","x.xml" to "<x/>"),
            mapOf("3D/3dmodel.model" to "<!DOCTYPE model [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><model>&x;</model>"),
            mapOf("Metadata/project_settings.config" to """{"printer_model":"Bambu Lab P1S","post_process":["/bin/sh"]}"""))) {
            project(entries) {assertThrows(BadRequestException::class.java) {inspector.inspect(it)}}
        }
    }
    @Test fun `valid model and empty scripts remain accepted`() {
        project(mapOf("3D/3dmodel.model" to "<model/>","Metadata/project_settings.config" to """{"printer_model":"Bambu Lab P1S","post_process":[]}""")) {
            val result=inspector.inspect(it);inspector.requireModel(result);assertEquals("Bambu Lab P1S",result.printerModel)
        }
    }
}
