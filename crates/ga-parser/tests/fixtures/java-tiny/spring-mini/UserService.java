package com.example.spring;

import java.util.Optional;
import com.example.util.*;

@Service
public class UserService {
    @Autowired
    private UserRepository userRepository;

    public User getUser(long id) {
        return userRepository.findById(id).orElseGet(() -> new User(id, "anonymous"));
    }

    public User register(String name) {
        User u = new User(System.currentTimeMillis(), name);
        return userRepository.save(u);
    }
}
