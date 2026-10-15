package com.goatpacker.web.server

import com.fasterxml.jackson.databind.ObjectMapper
import jakarta.servlet.FilterChain
import jakarta.servlet.http.*
import org.springframework.context.annotation.*
import org.springframework.boot.ApplicationRunner
import org.springframework.boot.autoconfigure.condition.ConditionalOnWebApplication
import org.springframework.boot.web.servlet.FilterRegistrationBean
import org.springframework.http.ResponseCookie
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.security.authentication.UsernamePasswordAuthenticationToken
import org.springframework.security.config.annotation.web.builders.HttpSecurity
import org.springframework.security.config.http.SessionCreationPolicy
import org.springframework.security.core.authority.SimpleGrantedAuthority
import org.springframework.security.core.context.SecurityContextHolder
import org.springframework.security.crypto.bcrypt.BCryptPasswordEncoder
import org.springframework.security.web.SecurityFilterChain
import org.springframework.security.web.authentication.UsernamePasswordAuthenticationFilter
import org.springframework.security.web.csrf.*
import org.springframework.stereotype.*
import org.springframework.transaction.annotation.Transactional
import org.springframework.web.bind.annotation.*
import org.springframework.web.filter.OncePerRequestFilter
import java.security.MessageDigest
import java.security.SecureRandom
import java.time.Instant
import java.util.Base64
import java.util.Locale

object Secrets {
    private val random = SecureRandom()
    fun token():String = Base64.getUrlEncoder().withoutPadding().encodeToString(ByteArray(32).also(random::nextBytes))
    fun hash(value:String):String = MessageDigest.getInstance("SHA-256").digest(value.toByteArray()).joinToString("") { "%02x".format(it) }
}
data class Actor(val id:String,val role:String,val kind:String,val login:String?)
fun actor():Actor = SecurityContextHolder.getContext().authentication?.principal as? Actor
    ?: throw ServerError(401,"UNAUTHORIZED","Нужна гостевая сессия или вход")

@Service @Profile("server")
class AdmissionLimits(private val jdbc:JdbcTemplate) {
    @Transactional fun hit(key:String, maximum:Int, seconds:Int=60) {
        val used=jdbc.queryForObject("""insert into request_limits(key,window_start,used) values (?,now(),1)
            on conflict(key) do update set
            used=case when request_limits.window_start < now()- (? * interval '1 second') then 1 else request_limits.used+1 end,
            window_start=case when request_limits.window_start < now()- (? * interval '1 second') then now() else request_limits.window_start end returning used""",Int::class.java,key,seconds,seconds)!!
        if(used>maximum) limited("Слишком много запросов — повторите позже")
    }
}

@Service @Profile("server")
class Identities(private val owners:PrincipalRepository,private val sessions:SessionRepository,
    private val resets:ResetRepository,private val platform:PlatformRepository,private val audit:AuditRepository,
    private val props:PlatformProperties, private val jdbc:JdbcTemplate) {
    private val encoder=BCryptPasswordEncoder(12)
    fun view(p:PrincipalEntity)=mapOf("id" to p.id,"kind" to p.kind,"login" to p.login,"role" to p.role,"blocked" to p.blocked,"storageQuotaBytes" to p.storageQuota)
    fun current():PrincipalEntity=owners.findById(actor().id).orElseThrow { ServerError(401,"UNAUTHORIZED","Сессия не найдена") }
    fun authenticate(token:String?):Actor? {
        if(token==null || token.length>100) return null
        val session=sessions.findById(Secrets.hash(token)).orElse(null) ?: return null
        val owner=owners.findById(session.ownerId).orElse(null) ?: return null
        if(owner.blocked) throw ServerError(403,"BLOCKED","Доступ заблокирован администратором")
        if(session.revoked || session.expiresAt.isBefore(Instant.now()) || owner.resetRequired) return null
        if(owner.lastSeenAt.isBefore(Instant.now().minusSeconds(60))) jdbc.update("update principals set last_seen_at=now() where id=? and last_seen_at<now()-interval '1 minute'",owner.id)
        return Actor(owner.id,owner.role,owner.kind,owner.login)
    }
    @Transactional fun bootstrap() {
        jdbc.update("insert into platform_config(id,max_parallel,paused,version) values (1,?,false,0) on conflict do nothing",props.maxParallel.coerceIn(0,64))
        platform.locked()
        if(props.adminLogin.isNotBlank() && owners.findByLogin(normalize(props.adminLogin))==null) {
            if(owners.count()>0 && jdbc.queryForObject("select count(*) from principals where role='ADMIN'",Long::class.java)!!>0) return
            validatePassword(props.adminPassword)
            val p=owners.save(PrincipalEntity(kind="USER",login=normalize(props.adminLogin),passwordHash=encoder.encode(props.adminPassword),role="ADMIN",storageQuota=1024L*1024*1024))
            audit.save(AuditEntity(targetId=p.id,action="ADMIN_BOOTSTRAP",details="Initial administrator created"))
        }
    }
    @Transactional fun guest(): Pair<PrincipalEntity,String> {
        platform.locked()
        if(owners.count()>=10000) limited("Лимит пользователей сервиса достигнут")
        val p=owners.save(PrincipalEntity());return p to session(p)
    }
    @Transactional fun register(login:String,password:String,current:Actor?):Pair<PrincipalEntity,String> {
        platform.locked(); val normalized=normalize(login);validatePassword(password)
        if(owners.findByLogin(normalized)!=null) bad("Это имя уже занято")
        val p=if(current?.kind=="GUEST") owners.locked(current.id) ?: missing() else {
            if(current!=null) bad("Сначала выйдите из текущего аккаунта")
            if(owners.count()>=10000) limited("Лимит пользователей сервиса достигнут")
            PrincipalEntity()
        }
        if(p.blocked) throw ServerError(403,"BLOCKED","Доступ заблокирован")
        p.kind="USER";p.login=normalized;p.passwordHash=encoder.encode(password);p.storageQuota=1024L*1024*1024
        owners.saveAndFlush(p); sessions.revoke(p.id)
        jdbc.update("delete from read_grants where job_id in (select id from queue_jobs where owner_id=?)",p.id)
        jdbc.update("update queue_jobs set expires_at=null,read_token_hash=null where owner_id=?",p.id)
        jdbc.update("update uploads set expires_at=null where owner_id=? and state='READY'",p.id)
        audit.save(AuditEntity(actorId=p.id,targetId=p.id,action="REGISTER",details="Guest content retained"))
        return p to session(p)
    }
    @Transactional fun login(login:String,password:String):Pair<PrincipalEntity,String> {
        if(password.length>128) bad("Некорректные данные входа")
        val p=owners.findByLogin(login.trim().lowercase(Locale.ROOT))
        val hash=p?.passwordHash ?: "\$2a\$12\$Ywrkc.hMnqcA84Pd7qxSLukuwMfTbXwWSxJrKOOhQczRb0HMk1WRK"
        val valid=encoder.matches(password,hash)
        if(p==null || !valid || p.resetRequired) throw ServerError(401,"UNAUTHORIZED","Неверные данные входа или требуется сброс пароля")
        if(p.blocked) throw ServerError(403,"BLOCKED","Доступ заблокирован")
        p.lastSeenAt=Instant.now();return p to session(p)
    }
    @Transactional fun logout(token:String?) { token?.let { sessions.findById(Secrets.hash(it)).orElse(null)?.let { s->s.revoked=true } } }
    @Transactional fun issueReset(target:String,reason:String):String {
        platform.locked(); val p=owners.locked(target) ?: missing()
        if(p.kind!="USER" || p.role=="ADMIN") bad("Сброс доступен только для обычного зарегистрированного пользователя")
        if(reason.trim().length !in 3..500) bad("Укажите причину (3–500 символов)")
        p.resetRequired=true;sessions.revoke(p.id);resets.removeFor(p.id)
        val token=Secrets.token();resets.save(ResetEntity(Secrets.hash(token),p.id,Instant.now().plusSeconds(900)))
        audit.save(AuditEntity(actorId=actor().id,targetId=p.id,action="PASSWORD_RESET_ISSUED",details=reason.trim()))
        return token
    }
    @Transactional fun consumeReset(token:String,password:String) {
        platform.locked();validatePassword(password)
        val r=resets.locked(Secrets.hash(token)) ?: bad("Ссылка сброса недействительна")
        if(r.usedAt!=null || r.expiresAt.isBefore(Instant.now())) bad("Ссылка сброса истекла или уже использована")
        val p=owners.locked(r.ownerId) ?: missing()
        if(p.blocked) throw ServerError(403,"BLOCKED","Доступ заблокирован")
        r.usedAt=Instant.now();p.passwordHash=encoder.encode(password);p.resetRequired=false;sessions.revoke(p.id)
        audit.save(AuditEntity(targetId=p.id,action="PASSWORD_RESET_COMPLETED",details="Sessions revoked"))
    }
    private fun session(p:PrincipalEntity):String {
        // Bound session count without exposing any token to logs.
        jdbc.update("delete from auth_sessions where owner_id=? and (revoked=true or expires_at<now())",p.id)
        if(jdbc.queryForObject("select count(*) from auth_sessions where owner_id=?",Long::class.java,p.id)!!>=10) sessions.revoke(p.id)
        val token=Secrets.token();sessions.save(SessionEntity(Secrets.hash(token),p.id,Instant.now().plusSeconds(7*86400)));return token
    }
    private fun normalize(login:String):String {
        val value=login.trim().lowercase(Locale.ROOT)
        if(!Regex("[a-z0-9][a-z0-9_.@+\\-]{2,119}").matches(value)) bad("Имя: 3–120 латинских букв, цифр или символов _.@+-")
        return value
    }
    private fun validatePassword(password:String) { if(password.length !in 12..72 || password.toByteArray().size>72) bad("Пароль: минимум 12 символов, максимум 72 байта") }
}

class SessionFilter(private val ids:Identities,private val mapper:ObjectMapper):OncePerRequestFilter() {
    override fun doFilterInternal(req:HttpServletRequest,res:HttpServletResponse,chain:FilterChain) {
        try {
            if(req.requestURI !in listOf("/api/v2/auth/logout","/api/v2/auth/csrf")) {
                ids.authenticate(req.cookies?.find { it.name=="GP_SESSION" }?.value)?.let {
                    SecurityContextHolder.getContext().authentication=UsernamePasswordAuthenticationToken(it,null,listOf(SimpleGrantedAuthority("ROLE_${it.role}")))
                }
            }
            chain.doFilter(req,res)
        } catch(ex:ServerError) {
            res.status=ex.status;res.contentType="application/json";mapper.writeValue(res.writer,mapOf("code" to ex.code,"message" to ex.message))
        }
    }
}
@Configuration @Profile("server") @ConditionalOnWebApplication
class ServerSecurity {
    @Bean fun serverChain(http:HttpSecurity,ids:Identities,mapper:ObjectMapper,props:PlatformProperties):SecurityFilterChain {
        val csrf=CookieCsrfTokenRepository.withHttpOnlyFalse().apply { setCookieCustomizer { it.secure(props.secureCookies).sameSite("Strict").path("/") } }
        return http.cors { }.sessionManagement { it.disable() }
            .securityContext { it.securityContextRepository(org.springframework.security.web.context.NullSecurityContextRepository()) }
            .requestCache { it.disable() }
            .csrf { it.csrfTokenRepository(csrf).csrfTokenRequestHandler(CsrfTokenRequestAttributeHandler()) }
            .addFilterBefore(SessionFilter(ids,mapper),UsernamePasswordAuthenticationFilter::class.java)
            .authorizeHttpRequests {
                it.requestMatchers("/api/v2/health","/api/v2/auth/csrf","/api/v2/auth/guest","/api/v2/auth/login","/api/v2/auth/register","/api/v2/auth/reset","/api/v2/auth/logout").permitAll()
                it.requestMatchers("/api/v2/admin/**").hasRole("ADMIN")
                it.requestMatchers("/api/v2/**").authenticated()
                it.anyRequest().denyAll()
            }.exceptionHandling {
                it.authenticationEntryPoint { _,res,_-> res.status=401;res.contentType="application/json";res.writer.write("""{"code":"UNAUTHORIZED","message":"Нужен вход или гостевая сессия"}""") }
                it.accessDeniedHandler { _,res,_->res.status=403;res.contentType="application/json";res.writer.write("""{"code":"FORBIDDEN","message":"Недостаточно прав или неверный CSRF-токен"}""") }
            }.build()
    }
}
@Configuration @Profile("server")
class IdentityBootstrap {
    @Bean fun bootstrapIdentity(ids:Identities)=ApplicationRunner { ids.bootstrap() }
}
@RestController @RequestMapping("/api/v2/auth") @Profile("server")
class AuthController(private val ids:Identities,private val props:PlatformProperties,private val limits:AdmissionLimits) {
    data class Credentials(val login:String="",val password:String="")
    data class ResetForm(val token:String="",val password:String="")
    @GetMapping("/csrf") fun csrf(token:CsrfToken)=mapOf("token" to token.token)
    @GetMapping("/me") fun me()=ids.view(ids.current())
    @PostMapping("/guest") fun guest(req:HttpServletRequest,res:HttpServletResponse):Map<String,Any?> {
        limits.hit("guest-global",60,60);limits.hit("guest:${req.remoteAddr}",10,3600)
        val current=SecurityContextHolder.getContext().authentication?.principal as? Actor
        if(current!=null) return ids.view(ids.current())
        return reply(ids.guest(),res)
    }
    @PostMapping("/register") fun register(@RequestBody form:Credentials,req:HttpServletRequest,res:HttpServletResponse):Map<String,Any?> {
        limits.hit("signup-global",10,60);limits.hit("signup:${req.remoteAddr}",5,3600)
        return reply(ids.register(form.login,form.password,SecurityContextHolder.getContext().authentication?.principal as? Actor),res)
    }
    @PostMapping("/login") fun login(@RequestBody form:Credentials,req:HttpServletRequest,res:HttpServletResponse):Map<String,Any?> {
        limits.hit("login-global",30,60);limits.hit("login:${req.remoteAddr}",10);return reply(ids.login(form.login,form.password),res)
    }
    @PostMapping("/logout") fun logout(req:HttpServletRequest,res:HttpServletResponse) {
        ids.logout(req.cookies?.find { it.name=="GP_SESSION" }?.value)
        res.addHeader("Set-Cookie",ResponseCookie.from("GP_SESSION","").httpOnly(true).secure(props.secureCookies).sameSite("Strict").path("/").maxAge(0).build().toString())
    }
    @PostMapping("/reset") fun reset(@RequestBody form:ResetForm,req:HttpServletRequest) { limits.hit("reset:${req.remoteAddr}",5);ids.consumeReset(form.token,form.password) }
    private fun reply(pair:Pair<PrincipalEntity,String>,res:HttpServletResponse):Map<String,Any?> {
        res.addHeader("Set-Cookie",ResponseCookie.from("GP_SESSION",pair.second).httpOnly(true).secure(props.secureCookies).sameSite("Strict").path("/").maxAge(7*86400).build().toString())
        return ids.view(pair.first)
    }
}
