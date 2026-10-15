package com.goatpacker.web.server

import org.springframework.context.annotation.Profile
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional
import org.springframework.web.bind.annotation.*
import java.security.MessageDigest
import java.time.Instant

/** One-use, owner-bound SHA-256 proof-of-work. A cost barrier, NOT a replacement for quotas. */
@Service @Profile("server")
class Challenges(private val jdbc:JdbcTemplate,private val limits:AdmissionLimits,private val props:PlatformProperties) {
    @Transactional fun issue(owner:String):Map<String,Any> {
        limits.hit("challenge-global",100,60);limits.hit("challenge:$owner",10,60)
        val id=newId();val salt=Secrets.token();val until=Instant.now().plusSeconds(180)
        jdbc.update("insert into bot_challenges(id,owner_id,salt,difficulty,expires_at) values (?,?,?,?,?)",id,owner,salt,props.powDifficulty,java.sql.Timestamp.from(until))
        return mapOf("id" to id,"salt" to salt,"difficulty" to props.powDifficulty,"expiresAt" to until,"algorithm" to "SHA-256")
    }
    @Transactional fun consume(owner:String,id:String?,nonce:String?) {
        if(!props.requireChallenge) return
        if(id==null || nonce==null || !Regex("[0-9]{1,16}").matches(nonce)) bad("Нужно пройти проверку перед загрузкой")
        val row=jdbc.queryForList("select owner_id,salt,difficulty,expires_at,used from bot_challenges where id=? for update",id).firstOrNull() ?: bad("Проверка истекла")
        if(row["owner_id"]!=owner || row["used"]==true || (row["expires_at"] as java.sql.Timestamp).toInstant().isBefore(Instant.now())) bad("Проверка истекла или уже использована")
        val digest=MessageDigest.getInstance("SHA-256").digest("$id:${row["salt"]}:$nonce".toByteArray())
        var bits=0
        for(byte in digest) { val n=byte.toInt() and 255; if(n==0) bits+=8 else {bits+=Integer.numberOfLeadingZeros(n)-24;break} }
        if(bits<(row["difficulty"] as Number).toInt()) bad("Некорректное подтверждение проверки")
        jdbc.update("update bot_challenges set used=true where id=?",id)
    }
}
@RestController @RequestMapping("/api/v2/challenges") @Profile("server")
class ChallengeController(private val challenges:Challenges) {
    @PostMapping fun challenge()=challenges.issue(actor().id)
}
