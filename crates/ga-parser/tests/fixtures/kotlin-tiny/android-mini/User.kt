package com.example

interface Printable {
    fun print()
}

open class User(val name: String) {
    fun describe(): String = "User($name)"
}
