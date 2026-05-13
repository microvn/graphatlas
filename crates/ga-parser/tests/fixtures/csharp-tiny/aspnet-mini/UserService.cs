using System;
using App.Models;
using App.Data;

namespace App.Services;

// Marker attribute used as Lang-C7 [Inject] sample (S-003c REFERENCES tests).
[AttributeUsage(AttributeTargets.Field | AttributeTargets.Property)]
public sealed class InjectAttribute : Attribute { }

public class UserService
{
    [Inject]
    private UserRepository _userRepository;

    public User GetUser(int id)
    {
        var cached = _userRepository.FindById(id);
        if (cached != null) return cached;
        var u = new User($"user-{id}");
        _userRepository.Save(u);
        return u;
    }
}

// C# 9+ partial class — exercises AS-011 (multi-file partial merging at indexer level).
public partial class PartialDemo
{
    public void MethodA() { Console.WriteLine("a"); }
}

public partial class PartialDemo
{
    public void MethodB() { Console.WriteLine("b"); }
}
