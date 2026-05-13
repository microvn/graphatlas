package com.example

import com.example.User

class UserRepository {
    private val cache: MutableMap<Int, User> = mutableMapOf()

    fun findById(id: Int): User? = cache[id]

    fun save(u: User) {
        cache[u.name.hashCode()] = u
    }

    suspend fun fetchRemote(id: Int): User {
        delay(10)
        return User("remote-$id")
    }
}

suspend fun delay(millis: Long) {
    // marker: suspend fn at top level
}
