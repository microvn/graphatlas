package com.example

import com.example.User
import com.example.UserRepository

class UserService {
    @Inject
    lateinit var userRepository: UserRepository

    fun getUser(id: Int): User {
        val cached = userRepository.findById(id)
        if (cached != null) return cached
        val u = User("user-$id")
        userRepository.save(u)
        return u
    }
}

fun String.isUserId(): Boolean = startsWith("user-")

annotation class Inject
