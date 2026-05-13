using System.Collections.Generic;
using System.Threading.Tasks;
using App.Models;

namespace App.Data;

public class UserRepository
{
    private readonly Dictionary<int, User> _cache = new Dictionary<int, User>();

    public User FindById(int id)
    {
        return _cache.TryGetValue(id, out var u) ? u : null;
    }

    public void Save(User u)
    {
        _cache[u.Name.GetHashCode()] = u;
    }

    public async Task<User> FetchRemoteAsync(int id)
    {
        await Task.Delay(10);
        return new User($"remote-{id}");
    }
}
