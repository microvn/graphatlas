package com.example.spring;

import java.util.Optional;

public interface UserRepository {
    Optional<User> findById(long id);

    User save(User user);
}
