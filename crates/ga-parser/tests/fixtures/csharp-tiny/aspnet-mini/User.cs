namespace App.Models;

public interface IPrintable
{
    void Print();
}

public class User
{
    public string Name { get; set; }

    public User(string name)
    {
        Name = name;
    }

    public string Describe() => $"User({Name})";
}
